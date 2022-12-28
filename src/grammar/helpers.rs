use crate::*;

pub(in crate) fn _check2arithmetic(mut x: Expr, mut y: Expr, op: BinOpType, ty: impl for <'a> FnOnce(&'a Expr) -> Type) -> Expr {
	assert!(x.ty.eq_implicit(&mut y.ty, Some(&mut x.kind), Some(&mut y.kind)), "cannot apply `{:?}` to different types", op);
	let ty = ty(&x);
	Expr {
		kind: ExprKind::BinOp {
			left: Box::new(x),
			right: Box::new(y),
			op,
		},
		ty
	}
}

pub(in crate) fn check2arithmetic(x: Expr, y: Expr, op: BinOpType) -> Expr {
	assert!(x.ty.is_arithmetic(), "cannot apply `{:?}` to non-arithmetic types", op);
	_check2arithmetic(x, y, op, |x| x.ty.clone())
}

pub(in crate) fn check2arithmetic_bool(x: Expr, y: Expr, op: BinOpType) -> Expr {
	_check2arithmetic(x, y, op, |_| Type::get_builtin("bool"))
}

pub(in crate) fn check2full_bool(x: Expr, y: Expr, op: BinOpType) -> Expr {
	let bool = Type::get_builtin("bool");
	assert_eq!(x.ty, bool, "cannot apply `{:?}` to non-boolean type", op);
	assert_eq!(y.ty, bool, "cannot apply `{:?}` to non-boolean type", op);
	_check2arithmetic(x, y, op, |_| bool)
}

pub(in crate) fn _get_static_method(mother: &Type, name: &str) -> Option <(FunLocation, &'static mut FunDef)> {
	let (index, method_index, method) = _get_fun_get_idx_and_method(&mother, name)?;
	assert_eq!(method.kind, AssociatedMethodKind::Static);
	Some((FunLocation::Method(FunMethodLocation {
		ty_index: TypeDefIndex { index },
		method_index,
	}), &mut method.def))
}

pub(in crate) fn access_field_inner(mother: Option <Result <Type, Expr>>, input: ParseFunBodyInput, mut fields: Vec <String>) -> Option <Expr> {
	let mut cur = match mother {
		Some(mother) => match mother {
			Ok(mother) => {
				// Check for static associated items
				let first = fields.remove(0);
				// TODO: replace that `expect` with check for static associated variables
				let (fun, def) = _get_static_method(&mother, &first)?;
				Expr {
					kind: ExprKind::FunCall {
						fun,
						args: vec![]
					},
					ty: def.ret_ty_as_determined(input, Some(AssociatedMethodKind::Static)).clone()
				}
			},
			Err(mother) => mother
		},
		_ => {
			let first = fields.remove(0);
			__expr1_variable(&first, input).ok().or_else(|| {
				let (stmt_index, fun) = input.fun_by_name_mut(&first)?;
				if !fun.args.is_empty() {
					return None
				}
				let ty = fun.ret_ty_as_determined(input, None).clone();
				Some(Expr {
					kind: ExprKind::FunCall {
						fun: FunLocation::Global { stmt_index },
						args: vec![],
					},
					ty,
				})
			})?
		}
	};

	for next in fields {
		cur = cur.dereference_if_ref_or_nop();
		if let Some((index, method_index, method)) = _get_fun_get_idx_and_method(&cur.ty, &next) {
			assert_eq!(method.def.args.len(), 0);
			let ty = method.def.ret_ty_as_determined(input, Some(method.kind)).clone();
			cur = Expr {
				kind: ExprKind::FunCall {
					fun: FunLocation::Method(FunMethodLocation {
						ty_index: TypeDefIndex { index },
						method_index,
					}),
					args: vec![cur],
				},
				ty,
			};
			continue
		}

		let fields = cur.ty.get_fields_of_struct().expect("cannot access a field of not-a-structure");
		let (idx, field) = fields.iter().enumerate().find(|(_, x)| x.name == next).expect("no field with such a name found");
		cur = Expr {
			kind: ExprKind::Variable {
				location: ExprKindVariableLocation::AccessField {
					i: Box::new(cur),
					def: fields,
					field: idx
				}
			},
			ty: field.ty.clone()
		}
	}

	Some(cur)
}

pub(in crate) fn _get_fun_get_idx_and_method(ty: &Type, method: &str) -> Option <(usize, usize, &'static mut AssociatedMethod)> {
	match ty.kind {
		TypeKind::Scalar { index } => match &mut Type::baked()[index].kind {
			BakedTypeKind::Ordinary(def) => def.methods.iter_mut().enumerate().find(|(_, x)| x.def.name == method)
				.map(|(a, b)| (index, a, b)),
			_ => None
		},
		_ => None
	}
}

pub(in crate) fn get_fun(mother: Option <Result <Type, Expr>>, input: ParseFunBodyInput, mut components: Vec <String>) -> Option <(FunLocation, &mut FunDef, usize, Option <Expr>)> {
	Some(if components.len() == 1 {
		if let Some(mother) = mother {
			match mother {
				Ok(mother) => {
					// Static method
					let (loc, method) = _get_static_method(&mother, &components[0])?;
					let len = method.args.len();
					(loc, method, len, None)
				},
				Err(expr) => {
					// Non-static method on `(expr)`
					let (index, method_index, method) = _get_fun_get_idx_and_method(&expr.ty, &components[0])?;
					let len = method.def.args.len();
					(FunLocation::Method(FunMethodLocation {
						ty_index: TypeDefIndex { index },
						method_index,
					}), &mut method.def, len, Some(expr))
				}
			}
		} else {
			// Global function
			let (i, d) = input.fun_by_name_mut(&components[0])?;
			let len = d.args.len();
			(FunLocation::Global { stmt_index: i }, d, len, None)
		}
	} else {
		let fun_name = components.pop().unwrap();
		let mother_ty = access_field_inner(mother, input, components)?.dereference_if_ref_or_nop();
		let (index, method_index, method) = _get_fun_get_idx_and_method(&mother_ty.ty, &fun_name)?;
		let len = method.def.args.len();
		(FunLocation::Method(FunMethodLocation {
			ty_index: TypeDefIndex { index },
			method_index,
		}), &mut method.def, len, Some(mother_ty))
	})
}

pub(in crate) fn assignment(input: ParseFunBodyInput, mut lvalue: Expr, mut new: Expr) -> (FunStmt, Type) {
	// if !lvalue.ty.is_copy() {
	//     // TODO! DROP lvalue and do not forget that drop may occur only on fully valid values
	// }

	assert!(lvalue.is_lvalue(input.stmts()), "cannot assign to a non-lvalue");
	new.mark_as_moved_and_panic_if_already(input);
	assert!(new.ty.eq_implicit(&mut lvalue.ty, Some(&mut new.kind), Some(&mut lvalue.kind)), "cannot assign two different types");
	(FunStmt::Assignment { lvalue, new }, Type::UNIT_TUPLE)
}

pub(in crate) fn dereference(ptr: Expr) -> Expr {
	let (ty, mutability) = match &ptr.ty.kind {
		TypeKind::Reference { ty, mutable } => ((**ty).clone(), *mutable),
		TypeKind::Pointer { ty, ptrs } => if ptrs.len == 1 {
			((**ty).clone(), (ptrs.muts & 1) != 0)
		} else {
			(Type::from_kind(TypeKind::Pointer {
				ty: ty.clone(),
				ptrs: TypePointers {
					len: ptrs.len - 1,
					muts: ptrs.muts << 1
				}
			}), (ptrs.muts & 1) != 0)
		},
		_ => panic!("cannot dereference a type that is neither pointer nor reference")
	};
	Expr {
		kind: ExprKind::Dereference {
			ptr: Box::new(ptr),
			may_be_mutable: mutability
		},
		ty,
	}
}
