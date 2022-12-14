use oko::*;
use std::ffi::CStr;

pub fn test_single_file(src: &str, result: &str) {
	create_llvm_context();
	create_llvm_module("oko");
	create_llvm_builder();

	let code = src.to_string() + "\n";

	let code = code.replace("    ", "\t");

	let code = remove_comments(code);

	Type::initialize_statics();

	let mut stmts = parse_raw_oko_code(&code).unwrap();

	bake_types();

	check_each_function_is_unique(&mut stmts);

	parse_body_in_each_function(&mut stmts);

	transpile_statements_into_llvm(&mut stmts);

	let res = unsafe { llvm::core::LLVMPrintModuleToString(llvm_module()) };
	let res = unsafe { CStr::from_ptr(res as _) };
	let res = res.to_str().unwrap();

	let res = normalize(res);
	let result = normalize(result);

	assert_eq!(res, result);

	drop_llvm_builder();
	drop_llvm_module();
	drop_llvm_context();
}

fn normalize(s: &str) -> String {
	s.replace("\n", "").replace(" ", "")
}
