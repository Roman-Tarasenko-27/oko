use llvm::core::*;
use llvm::prelude::*;
use crate::*;
use core::fmt::{Debug, Formatter, Write, Result};

#[derive(Debug, Clone)]
pub enum ExprKindVariableLocation {
	FunArg {
		fun_stmt_index: usize,
		fun_overload: usize,
		var_index: usize
	},
	Val {
		fun_stmt_index: usize,
		fun_overload: usize,
		line_def: usize
	}
}

#[derive(Copy, Clone, Eq, PartialEq)]
#[repr(u8)]
pub enum BinOpType {
	Add,
	Sub,
	Mul,
	Div
}

impl Debug for BinOpType {
	fn fmt(&self, f: &mut Formatter <'_>) -> Result {
		f.write_char(match self {
			Self::Add => '+',
			Self::Sub => '-',
			Self::Mul => '*',
			Self::Div => '/'
		})
	}
}

#[derive(Debug, Clone)]
#[repr(u8)]
pub enum ExprKind {
	Variable(ExprKindVariableLocation),
	Tuple(Vec <Expr>),
	FunCall {
		fun_stmt_index: usize,
		fun_overload: usize,
		args: Vec <Expr>
	},
	BinOp {
		left: Box <Expr>,
		right: Box <Expr>,
		op: BinOpType
	}
}

impl ExprKind {
	pub const UNIT_TUPLE: ExprKind = ExprKind::Tuple(vec![]);

	pub fn to_llvm_value(&self, stmts: &[Stmt]) -> LLVMValueRef {
		match self {
			Self::Variable(location) => build_variable(location, stmts),
			Self::Tuple(values) => build_tuple(values, stmts),
			Self::FunCall { fun_stmt_index, fun_overload, args }
				=> build_fun_call(*fun_stmt_index, *fun_overload, args, stmts),
			Self::BinOp { left, right, op }
				=> build_bin_op(left, right, *op, stmts)
		}
	}
}

fn build_bin_op(left0: &Expr, right0: &Expr, op: BinOpType, stmts: &[Stmt]) -> LLVMValueRef {
	let left = left0.kind.to_llvm_value(stmts);
	let right = right0.kind.to_llvm_value(stmts);
	let name = b"\0".as_ptr() as _;
	unsafe {
		match op {
			BinOpType::Add => LLVMBuildAdd(llvm_builder(), left, right, name),
			BinOpType::Sub => LLVMBuildSub(llvm_builder(), left, right, name),
			BinOpType::Mul => LLVMBuildMul(llvm_builder(), left, right, name),
			BinOpType::Div => if left0.ty.is_signed() {
				LLVMBuildSDiv(llvm_builder(), left, right, name)
			} else if left0.ty.is_unsigned() {
				LLVMBuildUDiv(llvm_builder(), left, right, name)
			} else {
				panic!("bad operand for division")
			},
		}
	}
}

fn build_tuple(values: &Vec <Expr>, stmts: &[Stmt]) -> LLVMValueRef {
	// TODO Rework!
	let mut values = values.iter().map(|x| x.kind.to_llvm_value(stmts)).collect::<Vec <_>>();
	unsafe { LLVMConstStructInContext(llvm_context(), values.as_mut_ptr(), values.len() as _, 0) }
}

fn build_variable(location: &ExprKindVariableLocation, stmts: &[Stmt]) -> LLVMValueRef {
	match location {
		ExprKindVariableLocation::FunArg {
			fun_stmt_index,
			fun_overload,
			var_index
		} => {
			let fun = match &stmts[*fun_stmt_index] {
				Stmt::FunDef(fun) => fun,
				_ => unreachable!()
			};
			let overload = &fun.overloads[*fun_overload];
			unsafe { LLVMGetParam(overload.llvm_fun.unwrap(), *var_index as _) }
		},
		ExprKindVariableLocation::Val {
			fun_stmt_index,
			fun_overload,
			line_def
		} => {
			let fun = match &stmts[*fun_stmt_index] {
				Stmt::FunDef(fun) => fun,
				_ => unreachable!()
			};
			let overload = &fun.overloads[*fun_overload];
			overload.vals.get(line_def).unwrap().llvm_value.unwrap()
		}
	}
}

fn build_fun_call(fun_stmt_index: usize, fun_overload: usize, args: &Vec <Expr>, stmts: &[Stmt]) -> LLVMValueRef {
	let fun = match &stmts[fun_stmt_index] {
		Stmt::FunDef(fun) => fun,
		_ => unreachable!()
	};
	let overload = &fun.overloads[fun_overload];
	let mut args = args.iter().map(|x| x.kind.to_llvm_value(stmts)).collect::<Vec <_>>();
	unsafe { LLVMBuildCall(llvm_builder(), overload.llvm_fun.unwrap(), args.as_mut_ptr(), args.len() as _, b"\0".as_ptr() as _) }
}

#[derive(Debug, Clone)]
pub struct Expr {
	pub kind: ExprKind,
	pub ty: Type
}

impl Expr {
	pub const UNIT_TUPLE: Expr = Expr {
		kind: ExprKind::UNIT_TUPLE,
		ty: Type::UNIT_TUPLE
	};
}
