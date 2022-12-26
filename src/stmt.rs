use crate::*;

#[derive(Debug)]
pub enum Stmt {
	TypeDef(TypeDefIndex),
	FunDef(FunDef),
	ExternFun(ExternFun)
}
