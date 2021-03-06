//! Builtin function definitions.
//!
//! Implements functions listed here: https://docs.python.org/3/library/builtins.html

// use std::ops::Deref;
use std::cell::RefCell;
use std::char;
use std::io::{self, Write};

use crate::compile;
use crate::obj::objbool;
use crate::obj::objdict;
use crate::obj::objint;
use crate::obj::objiter;
use crate::obj::objstr;
use crate::obj::objtype;

use crate::pyobject::{
    AttributeProtocol, IdProtocol, PyContext, PyFuncArgs, PyObject, PyObjectPayload, PyObjectRef,
    PyResult, Scope, TypeProtocol,
};

#[cfg(not(target_arch = "wasm32"))]
use crate::stdlib::io::io_open;

use crate::vm::VirtualMachine;
use num_traits::{Signed, ToPrimitive};

fn get_locals(vm: &mut VirtualMachine) -> PyObjectRef {
    let d = vm.new_dict();
    // TODO: implement dict_iter_items?
    let locals = vm.get_locals();
    let key_value_pairs = objdict::get_key_value_pairs(&locals);
    for (key, value) in key_value_pairs {
        objdict::set_item(&d, vm, &key, &value);
    }
    d
}

fn dir_locals(vm: &mut VirtualMachine) -> PyObjectRef {
    get_locals(vm)
}

fn dir_object(vm: &mut VirtualMachine, obj: &PyObjectRef) -> PyObjectRef {
    // Gather all members here:
    let attributes = objtype::get_attributes(obj);
    let mut members: Vec<String> = attributes.into_iter().map(|(n, _o)| n).collect();

    // Sort members:
    members.sort();

    let members_pystr = members.into_iter().map(|m| vm.ctx.new_str(m)).collect();
    vm.ctx.new_list(members_pystr)
}

fn builtin_abs(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None)]);
    match vm.get_method(x.clone(), "__abs__") {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![], vec![])),
        Err(..) => Err(vm.new_type_error("bad operand for abs".to_string())),
    }
}

fn builtin_all(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    for item in items {
        let result = objbool::boolval(vm, item)?;
        if !result {
            return Ok(vm.new_bool(false));
        }
    }
    Ok(vm.new_bool(true))
}

fn builtin_any(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    for item in items {
        let result = objbool::boolval(vm, item)?;
        if result {
            return Ok(vm.new_bool(true));
        }
    }
    Ok(vm.new_bool(false))
}

// builtin_ascii

fn builtin_bin(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0b{:b}", n.abs())
    } else {
        format!("0b{:b}", n)
    };

    Ok(vm.new_str(s))
}

// builtin_breakpoint

fn builtin_callable(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    let is_callable = obj.typ().has_attr("__call__");
    Ok(vm.new_bool(is_callable))
}

fn builtin_chr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(i, Some(vm.ctx.int_type()))]);

    let code_point = objint::get_value(i).to_u32().unwrap();

    let txt = match char::from_u32(code_point) {
        Some(value) => value.to_string(),
        None => '_'.to_string(),
    };

    Ok(vm.new_str(txt))
}

fn builtin_compile(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (source, None),
            (filename, Some(vm.ctx.str_type())),
            (mode, Some(vm.ctx.str_type()))
        ]
    );
    let source = objstr::get_value(source);
    // TODO: fix this newline bug:
    let source = format!("{}\n", source);

    let mode = {
        let mode = objstr::get_value(mode);
        if mode == "exec" {
            compile::Mode::Exec
        } else if mode == "eval" {
            compile::Mode::Eval
        } else if mode == "single" {
            compile::Mode::Single
        } else {
            return Err(
                vm.new_value_error("compile() mode must be 'exec', 'eval' or single'".to_string())
            );
        }
    };

    let filename = objstr::get_value(filename);

    compile::compile(&source, &mode, filename, vm.ctx.code_type()).map_err(|err| {
        let syntax_error = vm.context().exceptions.syntax_error.clone();
        vm.new_exception(syntax_error, err.to_string())
    })
}

fn builtin_delattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    vm.del_attr(obj, attr.clone())
}

fn builtin_dir(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.is_empty() {
        Ok(dir_locals(vm))
    } else {
        let obj = args.args.into_iter().next().unwrap();
        Ok(dir_object(vm, &obj))
    }
}

fn builtin_divmod(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(x, None), (y, None)]);
    match vm.get_method(x.clone(), "__divmod__") {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![y.clone()], vec![])),
        Err(..) => Err(vm.new_type_error("unsupported operand type(s) for divmod".to_string())),
    }
}

/// Implements `eval`.
/// See also: https://docs.python.org/3/library/functions.html#eval
fn builtin_eval(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.code_type()) {
        source.clone()
    } else if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Eval;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(&source, &mode, "<string>".to_string(), vm.ctx.code_type()).map_err(
            |err| {
                let syntax_error = vm.context().exceptions.syntax_error.clone();
                vm.new_exception(syntax_error, err.to_string())
            },
        )?
    } else {
        return Err(vm.new_type_error("code argument must be str or code object".to_string()));
    };

    let scope = make_scope(vm, locals);

    // Run the source:
    vm.run_code_obj(code_obj.clone(), scope)
}

/// Implements `exec`
/// https://docs.python.org/3/library/functions.html#exec
fn builtin_exec(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(source, None)],
        optional = [
            (_globals, Some(vm.ctx.dict_type())),
            (locals, Some(vm.ctx.dict_type()))
        ]
    );

    // Determine code object:
    let code_obj = if objtype::isinstance(source, &vm.ctx.str_type()) {
        let mode = compile::Mode::Exec;
        let source = objstr::get_value(source);
        // TODO: fix this newline bug:
        let source = format!("{}\n", source);
        compile::compile(&source, &mode, "<string>".to_string(), vm.ctx.code_type()).map_err(
            |err| {
                let syntax_error = vm.context().exceptions.syntax_error.clone();
                vm.new_exception(syntax_error, err.to_string())
            },
        )?
    } else if objtype::isinstance(source, &vm.ctx.code_type()) {
        source.clone()
    } else {
        return Err(vm.new_type_error("source argument must be str or code object".to_string()));
    };

    let scope = make_scope(vm, locals);

    // Run the code:
    vm.run_code_obj(code_obj, scope)
}

fn make_scope(vm: &mut VirtualMachine, locals: Option<&PyObjectRef>) -> PyObjectRef {
    // handle optional global and locals
    let locals = if let Some(locals) = locals {
        locals.clone()
    } else {
        vm.new_dict()
    };

    // TODO: handle optional globals
    // Construct new scope:
    let scope_inner = Scope {
        locals,
        parent: None,
    };

    PyObject {
        payload: PyObjectPayload::Scope {
            scope: RefCell::new(scope_inner),
        },
        typ: None,
    }
    .into_ref()
}

fn builtin_format(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None)],
        optional = [(format_spec, Some(vm.ctx.str_type()))]
    );
    let format_spec = format_spec
        .cloned()
        .unwrap_or_else(|| vm.new_str("".to_string()));
    vm.call_method(obj, "__format__", vec![format_spec])
}

fn builtin_getattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    vm.get_attribute(obj.clone(), attr.clone())
}

// builtin_globals

fn builtin_hasattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type()))]
    );
    let has_attr = match vm.get_attribute(obj.clone(), attr.clone()) {
        Ok(..) => true,
        Err(..) => false,
    };
    Ok(vm.context().new_bool(has_attr))
}

fn builtin_hash(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    vm.call_method(obj, "__hash__", vec![])
}

// builtin_help

fn builtin_hex(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0x{:x}", n.abs())
    } else {
        format!("0x{:x}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_id(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    Ok(vm.context().new_int(obj.get_id()))
}

// builtin_input

fn builtin_isinstance(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None), (typ, None)]);

    let isinstance = objtype::isinstance(obj, typ);
    Ok(vm.context().new_bool(isinstance))
}

fn builtin_issubclass(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    if args.args.len() != 2 {
        panic!("issubclass expects exactly two parameters");
    }

    let cls1 = &args.args[0];
    let cls2 = &args.args[1];

    Ok(vm.context().new_bool(objtype::issubclass(cls1, cls2)))
}

fn builtin_iter(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iter_target, None)]);
    objiter::get_iter(vm, iter_target)
}

fn builtin_len(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    let len_method_name = "__len__";
    match vm.get_method(obj.clone(), len_method_name) {
        Ok(value) => vm.invoke(value, PyFuncArgs::default()),
        Err(..) => Err(vm.new_type_error(format!(
            "object of type '{}' has no method {:?}",
            objtype::get_type_name(&obj.typ()),
            len_method_name
        ))),
    }
}

fn builtin_locals(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args);
    Ok(vm.get_locals())
}

fn builtin_max(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.is_empty() {
        let default = args.get_optional_kwarg("default");
        if default.is_none() {
            return Err(vm.new_value_error("max() arg is an empty sequence".to_string()));
        } else {
            return Ok(default.unwrap());
        }
    }

    let key_func = args.get_optional_kwarg("key");

    // Start with first assumption:
    let mut candidates_iter = candidates.into_iter();
    let mut x = candidates_iter.next().unwrap();
    // TODO: this key function looks pretty duplicate. Maybe we can create
    // a local function?
    let mut x_key = if let Some(f) = &key_func {
        let args = PyFuncArgs::new(vec![x.clone()], vec![]);
        vm.invoke(f.clone(), args)?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            let args = PyFuncArgs::new(vec![y.clone()], vec![]);
            vm.invoke(f.clone(), args)?
        } else {
            y.clone()
        };
        let order = vm._gt(x_key.clone(), y_key.clone())?;

        if !objbool::get_value(&order) {
            x = y.clone();
            x_key = y_key;
        }
    }

    Ok(x)
}

fn builtin_min(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    let candidates = if args.args.len() > 1 {
        args.args.clone()
    } else if args.args.len() == 1 {
        vm.extract_elements(&args.args[0])?
    } else {
        // zero arguments means type error:
        return Err(vm.new_type_error("Expected 1 or more arguments".to_string()));
    };

    if candidates.is_empty() {
        let default = args.get_optional_kwarg("default");
        if default.is_none() {
            return Err(vm.new_value_error("min() arg is an empty sequence".to_string()));
        } else {
            return Ok(default.unwrap());
        }
    }

    let key_func = args.get_optional_kwarg("key");

    let mut candidates_iter = candidates.into_iter();
    let mut x = candidates_iter.next().unwrap();
    // TODO: this key function looks pretty duplicate. Maybe we can create
    // a local function?
    let mut x_key = if let Some(f) = &key_func {
        let args = PyFuncArgs::new(vec![x.clone()], vec![]);
        vm.invoke(f.clone(), args)?
    } else {
        x.clone()
    };

    for y in candidates_iter {
        let y_key = if let Some(f) = &key_func {
            let args = PyFuncArgs::new(vec![y.clone()], vec![]);
            vm.invoke(f.clone(), args)?
        } else {
            y.clone()
        };
        let order = vm._gt(x_key.clone(), y_key.clone())?;

        if objbool::get_value(&order) {
            x = y.clone();
            x_key = y_key;
        }
    }

    Ok(x)
}

fn builtin_next(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(iterator, None)],
        optional = [(default_value, None)]
    );

    match vm.call_method(iterator, "__next__", vec![]) {
        Ok(value) => Ok(value),
        Err(value) => {
            if objtype::isinstance(&value, &vm.ctx.exceptions.stop_iteration) {
                match default_value {
                    None => Err(value),
                    Some(value) => Ok(value.clone()),
                }
            } else {
                Err(value)
            }
        }
    }
}

fn builtin_oct(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(number, Some(vm.ctx.int_type()))]);

    let n = objint::get_value(number);
    let s = if n.is_negative() {
        format!("-0o{:o}", n.abs())
    } else {
        format!("0o{:o}", n)
    };

    Ok(vm.new_str(s))
}

fn builtin_ord(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(string, Some(vm.ctx.str_type()))]);
    let string = objstr::get_value(string);
    let string_len = string.chars().count();
    if string_len > 1 {
        return Err(vm.new_type_error(format!(
            "ord() expected a character, but string of length {} found",
            string_len
        )));
    }
    match string.chars().next() {
        Some(character) => Ok(vm.context().new_int(character as i32)),
        None => Err(vm.new_type_error(
            "ord() could not guess the integer representing this character".to_string(),
        )),
    }
}

fn builtin_pow(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(x, None), (y, None)],
        optional = [(mod_value, Some(vm.ctx.int_type()))]
    );
    let pow_method_name = "__pow__";
    let result = match vm.get_method(x.clone(), pow_method_name) {
        Ok(attrib) => vm.invoke(attrib, PyFuncArgs::new(vec![y.clone()], vec![])),
        Err(..) => Err(vm.new_type_error("unsupported operand type(s) for pow".to_string())),
    };
    //Check if the 3rd argument is defined and perform modulus on the result
    //this should be optimized in the future to perform a "power-mod" algorithm in
    //order to improve performance
    match mod_value {
        Some(mod_value) => {
            let mod_method_name = "__mod__";
            match vm.get_method(result.expect("result not defined").clone(), mod_method_name) {
                Ok(value) => vm.invoke(value, PyFuncArgs::new(vec![mod_value.clone()], vec![])),
                Err(..) => {
                    Err(vm.new_type_error("unsupported operand type(s) for mod".to_string()))
                }
            }
        }
        None => result,
    }
}

pub fn builtin_print(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    trace!("print called with {:?}", args);

    // Handle 'sep' kwarg:
    let sep_arg = args
        .get_optional_kwarg("sep")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = sep_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "sep must be None or a string, not {}",
                objtype::get_type_name(&obj.typ())
            )));
        }
    }
    let sep_str = sep_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // Handle 'end' kwarg:
    let end_arg = args
        .get_optional_kwarg("end")
        .filter(|obj| !obj.is(&vm.get_none()));
    if let Some(ref obj) = end_arg {
        if !objtype::isinstance(obj, &vm.ctx.str_type()) {
            return Err(vm.new_type_error(format!(
                "end must be None or a string, not {}",
                objtype::get_type_name(&obj.typ())
            )));
        }
    }
    let end_str = end_arg.as_ref().map(|obj| objstr::borrow_value(obj));

    // Handle 'flush' kwarg:
    let flush = if let Some(flush) = &args.get_optional_kwarg("flush") {
        objbool::boolval(vm, flush.clone()).unwrap()
    } else {
        false
    };

    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();
    let mut first = true;
    for a in &args.args {
        if first {
            first = false;
        } else if let Some(ref sep_str) = sep_str {
            write!(stdout_lock, "{}", sep_str).unwrap();
        } else {
            write!(stdout_lock, " ").unwrap();
        }
        let v = vm.to_str(&a)?;
        let s = objstr::borrow_value(&v);
        write!(stdout_lock, "{}", s).unwrap();
    }

    if let Some(end_str) = end_str {
        write!(stdout_lock, "{}", end_str).unwrap();
    } else {
        writeln!(stdout_lock).unwrap();
    }

    if flush {
        stdout_lock.flush().unwrap();
    }

    Ok(vm.get_none())
}

fn builtin_repr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);
    vm.to_repr(obj)
}

fn builtin_reversed(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(obj, None)]);

    match vm.get_method(obj.clone(), "__reversed__") {
        Ok(value) => vm.invoke(value, PyFuncArgs::default()),
        // TODO: fallback to using __len__ and __getitem__, if object supports sequence protocol
        Err(..) => Err(vm.new_type_error(format!(
            "'{}' object is not reversible",
            objtype::get_type_name(&obj.typ()),
        ))),
    }
}
// builtin_reversed

fn builtin_round(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(number, Some(vm.ctx.object()))],
        optional = [(ndigits, None)]
    );
    if let Some(ndigits) = ndigits {
        let ndigits = vm.call_method(ndigits, "__int__", vec![])?;
        let rounded = vm.call_method(number, "__round__", vec![ndigits])?;
        Ok(rounded)
    } else {
        // without a parameter, the result type is coerced to int
        let rounded = &vm.call_method(number, "__round__", vec![])?;
        Ok(vm.ctx.new_int(objint::get_value(rounded)))
    }
}

fn builtin_setattr(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(obj, None), (attr, Some(vm.ctx.str_type())), (value, None)]
    );
    let name = objstr::get_value(attr);
    vm.ctx.set_attr(obj, &name, value.clone());
    Ok(vm.get_none())
}

// builtin_slice

fn builtin_sorted(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;
    let lst = vm.ctx.new_list(items);

    args.shift();
    vm.call_method_pyargs(&lst, "sort", args)?;
    Ok(lst)
}

fn builtin_sum(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(iterable, None)]);
    let items = vm.extract_elements(iterable)?;

    // Start with zero and add at will:
    let mut sum = vm.ctx.new_int(0);
    for item in items {
        sum = vm._add(sum, item)?;
    }
    Ok(sum)
}

// builtin_vars
// builtin___import__

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = py_module!(ctx, "__builtins__", {
        //set __name__ fixes: https://github.com/RustPython/RustPython/issues/146
        "__name__" => ctx.new_str(String::from("__main__")),

        "abs" => ctx.new_rustfunc(builtin_abs),
        "all" => ctx.new_rustfunc(builtin_all),
        "any" => ctx.new_rustfunc(builtin_any),
        "bin" => ctx.new_rustfunc(builtin_bin),
        "bool" => ctx.bool_type(),
        "bytearray" => ctx.bytearray_type(),
        "bytes" => ctx.bytes_type(),
        "callable" => ctx.new_rustfunc(builtin_callable),
        "chr" => ctx.new_rustfunc(builtin_chr),
        "classmethod" => ctx.classmethod_type(),
        "compile" => ctx.new_rustfunc(builtin_compile),
        "complex" => ctx.complex_type(),
        "delattr" => ctx.new_rustfunc(builtin_delattr),
        "dict" => ctx.dict_type(),
        "divmod" => ctx.new_rustfunc(builtin_divmod),
        "dir" => ctx.new_rustfunc(builtin_dir),
        "enumerate" => ctx.enumerate_type(),
        "eval" => ctx.new_rustfunc(builtin_eval),
        "exec" => ctx.new_rustfunc(builtin_exec),
        "float" => ctx.float_type(),
        "frozenset" => ctx.frozenset_type(),
        "filter" => ctx.filter_type(),
        "format" => ctx.new_rustfunc(builtin_format),
        "getattr" => ctx.new_rustfunc(builtin_getattr),
        "hasattr" => ctx.new_rustfunc(builtin_hasattr),
        "hash" => ctx.new_rustfunc(builtin_hash),
        "hex" => ctx.new_rustfunc(builtin_hex),
        "id" => ctx.new_rustfunc(builtin_id),
        "int" => ctx.int_type(),
        "isinstance" => ctx.new_rustfunc(builtin_isinstance),
        "issubclass" => ctx.new_rustfunc(builtin_issubclass),
        "iter" => ctx.new_rustfunc(builtin_iter),
        "len" => ctx.new_rustfunc(builtin_len),
        "list" => ctx.list_type(),
        "locals" => ctx.new_rustfunc(builtin_locals),
        "map" => ctx.map_type(),
        "max" => ctx.new_rustfunc(builtin_max),
        "memoryview" => ctx.memoryview_type(),
        "min" => ctx.new_rustfunc(builtin_min),
        "object" => ctx.object(),
        "oct" => ctx.new_rustfunc(builtin_oct),
        "ord" => ctx.new_rustfunc(builtin_ord),
        "next" => ctx.new_rustfunc(builtin_next),
        "pow" => ctx.new_rustfunc(builtin_pow),
        "print" => ctx.new_rustfunc(builtin_print),
        "property" => ctx.property_type(),
        "range" => ctx.range_type(),
        "repr" => ctx.new_rustfunc(builtin_repr),
        "reversed" => ctx.new_rustfunc(builtin_reversed),
        "round" => ctx.new_rustfunc(builtin_round),
        "set" => ctx.set_type(),
        "setattr" => ctx.new_rustfunc(builtin_setattr),
        "sorted" => ctx.new_rustfunc(builtin_sorted),
        "slice" => ctx.slice_type(),
        "staticmethod" => ctx.staticmethod_type(),
        "str" => ctx.str_type(),
        "sum" => ctx.new_rustfunc(builtin_sum),
        "super" => ctx.super_type(),
        "tuple" => ctx.tuple_type(),
        "type" => ctx.type_type(),
        "zip" => ctx.zip_type(),

        // Constants
        "NotImplemented" => ctx.not_implemented.clone(),

        // Exceptions:
        "BaseException" => ctx.exceptions.base_exception_type.clone(),
        "Exception" => ctx.exceptions.exception_type.clone(),
        "ArithmeticError" => ctx.exceptions.arithmetic_error.clone(),
        "AssertionError" => ctx.exceptions.assertion_error.clone(),
        "AttributeError" => ctx.exceptions.attribute_error.clone(),
        "NameError" => ctx.exceptions.name_error.clone(),
        "OverflowError" => ctx.exceptions.overflow_error.clone(),
        "RuntimeError" => ctx.exceptions.runtime_error.clone(),
        "NotImplementedError" => ctx.exceptions.not_implemented_error.clone(),
        "TypeError" => ctx.exceptions.type_error.clone(),
        "ValueError" => ctx.exceptions.value_error.clone(),
        "IndexError" => ctx.exceptions.index_error.clone(),
        "ImportError" => ctx.exceptions.import_error.clone(),
        "FileNotFoundError" => ctx.exceptions.file_not_found_error.clone(),
        "StopIteration" => ctx.exceptions.stop_iteration.clone(),
        "ZeroDivisionError" => ctx.exceptions.zero_division_error.clone(),
        "KeyError" => ctx.exceptions.key_error.clone(),
    });

    #[cfg(not(target_arch = "wasm32"))]
    ctx.set_attr(&py_mod, "open", ctx.new_rustfunc(io_open));

    py_mod
}

pub fn builtin_build_class_(vm: &mut VirtualMachine, mut args: PyFuncArgs) -> PyResult {
    let function = args.shift();
    let name_arg = args.shift();
    let bases = args.args.clone();
    let mut metaclass = args.get_kwarg("metaclass", vm.get_type());

    for base in bases.clone() {
        if objtype::issubclass(&base.typ(), &metaclass) {
            metaclass = base.typ();
        } else if !objtype::issubclass(&metaclass, &base.typ()) {
            return Err(vm.new_type_error("metaclass conflict: the metaclass of a derived class must be a (non-strict) subclass of the metaclasses of all its bases".to_string()));
        }
    }

    let bases = vm.context().new_tuple(bases);

    // Prepare uses full __getattribute__ resolution chain.
    let prepare_name = vm.new_str("__prepare__".to_string());
    let prepare = vm.get_attribute(metaclass.clone(), prepare_name)?;
    let namespace = vm.invoke(
        prepare,
        PyFuncArgs {
            args: vec![name_arg.clone(), bases.clone()],
            kwargs: vec![],
        },
    )?;

    vm.invoke(
        function,
        PyFuncArgs {
            args: vec![namespace.clone()],
            kwargs: vec![],
        },
    )?;

    vm.call_method(&metaclass, "__call__", vec![name_arg, bases, namespace])
}
