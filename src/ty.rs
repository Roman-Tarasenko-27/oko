use core::fmt::{Debug, Formatter, Result, Write};
use std::ffi::CString;
use llvm::core::*;
use llvm::prelude::*;
use crate::*;

#[derive(Debug)]
pub struct TypeDef {
	pub name: String,
	pub kind: TypeDefKind
}

#[derive(Copy, Clone)]
pub struct TypeDefIndex {
	/// Index of a type definition in a type list
	pub index: usize
}

impl Debug for TypeDefIndex {
	fn fmt(&self, f: &mut Formatter <'_>) -> Result {
		match Type::type_list() {
			TypeList::Raw(raw) => raw[self.index].as_backed(),
			TypeList::Baked(baked) => baked[self.index].as_ordinary()
		}.fmt(f)
	}
}

#[derive(Debug)]
pub enum TypeDefKind {
	Enum {
		variants: Vec <EnumVariant>
	},
	Struct {
		fields: Vec <StructField>
	},
	/// Effectively a struct or enum with no fields; an empty type
	Opaque
}

#[derive(Debug)]
pub struct EnumVariant {
	pub name: String,
	pub data: Vec <Type>
}

#[derive(Debug)]
pub struct StructField {
	pub name: String,
	pub ty: Type
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct TypePointers {
	/// How many pointers are on a type
	///
	/// **T -> 2
	/// *^^**^T -> 6
	/// T -> 0
	pub len: u8,

	/// How much of those pointers are mutable
	/// **T -> 0b00
	/// ^T -> 0b1
	/// *^^**^T -> 0b011001
	pub muts: u8
}

impl Debug for TypePointers {
	fn fmt(&self, f: &mut Formatter <'_>) -> Result {
		for i in 0..self.len {
			let is_mutable = ((self.muts >> i) & 1) != 0;
			f.write_char(if is_mutable {
				'^'
			} else {
				'*'
			})?
		}
		Ok(())
	}
}

impl TypePointers {
	pub fn from(str: &str) -> Self {
		if str.len() > 8 {
			panic!("no more than 8 pointers at a time are allowed on a type")
		}

		let len = str.len() as u8;
		let mut muts = 0b0000_0000;
		for (idx, char) in str.chars().enumerate() {
			if char == '^' {
				muts |= 1 << idx
			}
		}

		Self {
			len,
			muts
		}
	}
}

#[derive(Eq, Clone)]
pub struct Type {
	pub kind: TypeKind,
	llvm_type: Option <LLVMTypeRef>
}

impl PartialEq for Type {
	fn eq(&self, other: &Self) -> bool {
		self.kind.eq(&other.kind)
	}
}

#[derive(Eq, PartialEq, Clone)]
pub enum TypeKind {
	Scalar {
		index: usize,
		ptrs: TypePointers
	},
	Tuple { types: Vec <Type> },
	Integer
}

impl Debug for Type {
	fn fmt(&self, f: &mut Formatter <'_>) -> Result {
		match &self.kind {
			TypeKind::Scalar {
				index,
				ptrs
			} => {
				ptrs.fmt(f)?;
				match Self::type_list() {
					TypeList::Raw(raw) => f.write_str(raw[*index].name()),
					TypeList::Baked(baked) => f.write_str(baked[*index].name())
				}
			},
			TypeKind::Tuple { types } => {
				f.write_char('(')?;
				for ty in types.iter().rev().skip(1).rev() {
					ty.fmt(f)?;
					f.write_str(", ")?
				}
				if let Some(last) = types.last() {
					last.fmt(f)?;
					if types.len() == 1 {
						f.write_char(',')?
					}
				}
				f.write_char(')')
			},
			TypeKind::Integer => f.write_str("{{integer}}")
		}
	}
}

impl Type {
	pub const UNIT_TUPLE: Type = Type::from_kind(TypeKind::Tuple { types: vec![] });

	pub fn get_builtin(name: &str) -> Self {
		match Self::type_list() {
			TypeList::Baked(baked) => {
				Self::from_kind(TypeKind::Scalar {
					index: baked.iter().enumerate().find(|(_, x)| x.name() == name).unwrap().0,
					ptrs: TypePointers {
						len: 0,
						muts: 0
					},
				})
			},
			_ => unimplemented!()
		}
	}

	pub fn eq_implicit(&mut self, other: &mut Self, expr1: Option <&mut ExprKind>, expr2: Option <&mut ExprKind>) -> bool {
		if self == other {
			return true
		}

		if !self.try_implicitly_convert(other, expr1) {
			other.try_implicitly_convert(self, expr2);
		}

		self == other
	}

	pub fn try_implicitly_convert(&mut self, other: &Self, expr: Option <&mut ExprKind>) -> bool {
		if !other.is_signed() && !other.is_unsigned() {
			return false
		}

		match &self.kind {
			TypeKind::Integer => match &other.kind {
				TypeKind::Integer => false,
				_ => {
					*self = other.clone();
					if let Some(expr) = expr {
						match expr {
							ExprKind::BinOp { left, right, .. } => {
								left.ty.try_implicitly_convert(other, Some(&mut left.kind));
								right.ty.try_implicitly_convert(other, Some(&mut right.kind));
							},
							_ => ()
						}
					}
					true
				}
			},
			_ => false
		}
	}

	fn __is_builtin_smth(&self, f: for <'a> fn(&'a BuiltinTypeKind) -> bool) -> bool {
		match &self.kind {
			TypeKind::Scalar { index, ptrs } => if ptrs.len != 0 {
				false
			} else {
				match Self::type_list() {
					TypeList::Baked(baked) => match &baked[*index].kind {
						BakedTypeKind::Builtin(idx) => f(&BUILTIN_TYPES[*idx].kind),
						_ => false
					},
					_ => unimplemented!()
				}
			},
			_ => false
		}
	}

	pub fn is_signed(&self) -> bool {
		self.__is_builtin_smth(|x| matches!(x, BuiltinTypeKind::Signed))
	}

	pub fn is_unsigned(&self) -> bool {
		self.__is_builtin_smth(|x| matches!(x, BuiltinTypeKind::Unsigned))
	}

	pub fn is_arithmetic(&self) -> bool {
		self.is_signed() || self.is_unsigned() || matches!(self.kind, TypeKind::Integer)
	}

	pub fn llvm_type(&self) -> LLVMTypeRef {
		if let Some(ty) = self.llvm_type {
			return ty
		}

		match &self.kind {
			TypeKind::Scalar { index, ptrs } => {
				let mut ty = match Self::type_list() {
					TypeList::Baked(baked) => baked[*index].llvm_type,
					TypeList::Raw(_) => unimplemented!()
				};
				for _ in 0..ptrs.len {
					// No distinction between mutable and const pointers
					ty = unsafe { LLVMPointerType(ty, 0) }
				}
				ty
			},
			TypeKind::Tuple { types } => {
				let mut types = types.iter().map(|x| x.llvm_type()).collect::<Vec <_>>();
				unsafe { LLVMStructType(types.as_mut_ptr(), types.len() as _, 0) }
			},
			TypeKind::Integer => Self::get_builtin("i32").llvm_type()
		}
	}

	pub fn type_list() -> &'static mut TypeList {
		static mut TYPE_LIST: TypeList = TypeList::Raw(vec![]);
		unsafe { &mut TYPE_LIST }
	}

	pub fn raw() -> &'static mut Vec <RawType> {
		match Self::type_list() {
			TypeList::Raw(raw) => raw,
			_ => unimplemented!()
		}
	}

	pub fn as_scalar_index(&self) -> usize {
		match &self.kind {
			TypeKind::Scalar { index, .. } => *index,
			_ => unimplemented!()
		}
	}

	pub fn meet_new_raw_scalar(ptrs: &str, name: String, typedef: Option <TypeDef>) -> Self {
		Self::from_kind(TypeKind::Scalar {
			index: match Self::raw().iter_mut().enumerate().find(|(_, ty)| ty.name() == name) {
				Some((idx, ty)) => {
					if let Some(typedef) = typedef {
						if ty.is_backed() {
							panic!("type `{name}` already exists")
						} else {
							*ty = RawType::Backed(typedef)
						}
					}
					idx
				},
				None => {
					Self::raw().push(match typedef {
						Some(x) => RawType::Backed(x),
						None => RawType::Stub(name)
					});
					Self::raw().len() - 1
				}
			},
			ptrs: TypePointers::from(ptrs)
		})
	}

	pub const fn from_kind(kind: TypeKind) -> Self {
		Self {
			kind,
			llvm_type: None
		}
	}
}

#[derive(Debug)]
pub enum TypeList {
	Raw(Vec <RawType>),
	Baked(Vec <BakedType>)
}

#[derive(Debug)]
pub struct BakedType {
	pub kind: BakedTypeKind,
	pub llvm_type: LLVMTypeRef
}

impl BakedType {
	pub const STUB: BakedType = BakedType {
		kind: BakedTypeKind::Builtin(0),
		llvm_type: core::ptr::null_mut()
	};

	pub fn builtin(idx: usize) -> Self {
		let llvm_type = unsafe { (BUILTIN_TYPES[idx].llvm_create)(llvm_context()) };

		Self {
			kind: BakedTypeKind::Builtin(idx),
			llvm_type
		}
	}

	pub fn ordinary(td: TypeDef) -> Self {
		let name = CString::new(td.name.as_str()).unwrap();
		let llvm_type = unsafe { LLVMStructCreateNamed(llvm_context(), name.as_ptr()) };

		Self {
			kind: BakedTypeKind::Ordinary(td),
			llvm_type
		}
	}

	pub fn name(&self) -> &str {
		match &self.kind {
			BakedTypeKind::Builtin(builtin) => BUILTIN_TYPES[*builtin].name,
			BakedTypeKind::Ordinary(ord) => &ord.name
		}
	}

	pub fn as_ordinary(&self) -> &TypeDef {
		match &self.kind {
			BakedTypeKind::Ordinary(x) => x,
			_ => unimplemented!()
		}
	}
}

#[derive(Debug)]
pub enum BakedTypeKind {
	Builtin(usize),
	Ordinary(TypeDef)
}

#[derive(Debug)]
pub enum RawType {
	Stub(String),
	Backed(TypeDef)
}

impl RawType {
	pub fn name(&self) -> &str {
		match self {
			Self::Stub(name) => name,
			Self::Backed(typedef) => &typedef.name
		}
	}

	pub fn is_backed(&self) -> bool {
		matches!(self, Self::Backed(_))
	}

	pub fn as_backed(&self) -> &TypeDef {
		match self {
			Self::Backed(x) => x,
			_ => unimplemented!()
		}
	}
}
