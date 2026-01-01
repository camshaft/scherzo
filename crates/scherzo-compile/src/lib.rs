use anyhow::{Context, Result, anyhow, bail};
use heck::ToKebabCase;
use ryu::Buffer;
use scherzo_gcode::{Number, Statement, Value, Word, parse};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, Ieee64, ImportSection, Instruction, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};
use wit_component::{ComponentEncoder, StringEncoding, embed_component_metadata};
use wit_encoder::{
    Interface, Package, PackageName, ResourceFunc, StandaloneFunc, Type, TypeDef, World,
};
use wit_parser::Resolve;

/// Result of compiling a G-code job.
#[derive(Debug, Clone)]
pub struct Compilation {
    /// Rendered WIT document describing the per-job host interface.
    pub wit: String,
    /// Core WebAssembly module that calls into host builder imports in-order.
    pub wasm: Vec<u8>,
    /// Component-encoded wasm with embedded WIT.
    pub component: Vec<u8>,
}

/// Compile a G-code program into a per-job WIT description and a wasm module
/// that calls host-provided builder functions in the same order as the input.
pub fn compile_gcode(source: &str) -> Result<Compilation> {
    let statements = parse(source).context("failed to parse gcode")?;
    let (verb_shapes, compiled_stmts) = infer_shapes(&statements)?;

    let wit = build_wit(&verb_shapes)?;
    let module = build_wasm(&verb_shapes, &compiled_stmts)?;
    let component = build_component(&wit, &module)?;
    let wasm = module.finish();

    Ok(Compilation {
        wit,
        wasm,
        component,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ParamKind {
    Int,
    Float,
    String,
    ListInt,
    ListFloat,
    ListString,
}

#[derive(Debug, Clone)]
enum ParamLiteral {
    I64(i64),
    F64(f64),
    Str(String),
    ListI64(Vec<i64>),
    ListF64(Vec<f64>),
    ListStr(Vec<String>),
}

#[derive(Debug, Clone)]
struct ParamShape {
    kinds: BTreeSet<ParamKind>,
}

#[derive(Debug, Clone)]
struct VerbShape {
    /// Original verb token, e.g. "G1" or "M104".
    raw: String,
    params: BTreeMap<String, ParamShape>,
}

#[derive(Debug, Clone)]
struct CompiledStatement {
    verb: String,
    params: Vec<(String, ParamLiteral)>,
}

fn infer_shapes(statements: &[Statement]) -> Result<(Vec<VerbShape>, Vec<CompiledStatement>)> {
    let mut per_verb: HashMap<String, VerbShape> = HashMap::new();
    let mut compiled = Vec::new();

    for stmt in statements {
        let Some((verb, tail)) = split_verb(stmt) else {
            continue;
        };

        let verb_shape = per_verb
            .entry(verb.raw.clone())
            .or_insert_with(|| VerbShape {
                raw: verb.raw.clone(),
                params: BTreeMap::new(),
            });

        let mut compiled_params = Vec::new();

        for word in tail {
            let Some((name, value)) = normalize_param(word) else {
                continue;
            };

            let (kind, literal) = classify_value(value)?;
            let shape = verb_shape
                .params
                .entry(name.clone())
                .or_insert_with(|| ParamShape {
                    kinds: BTreeSet::new(),
                });
            shape.kinds.insert(kind.clone());
            compiled_params.push((name, literal));
        }

        compiled.push(CompiledStatement {
            verb: verb.raw,
            params: compiled_params,
        });
    }

    let mut verbs: Vec<_> = per_verb.into_values().collect();
    verbs.sort_by(|a, b| a.raw.cmp(&b.raw));
    Ok((verbs, compiled))
}

fn split_verb(stmt: &Statement) -> Option<(NormalizedVerb, &[Word])> {
    let first = stmt.words.first()?;
    let verb = normalize_verb(first)?;
    Some((verb, &stmt.words[1..]))
}

#[derive(Debug, Clone)]
struct NormalizedVerb {
    raw: String,
}

fn normalize_verb(word: &Word) -> Option<NormalizedVerb> {
    if let Some(name) = &word.name {
        return Some(NormalizedVerb { raw: name.clone() });
    }

    let letter = word.letter?;
    let raw = match &word.value {
        Some(Value::Number(Number::Int(i))) => format!("{letter}{i}"),
        Some(Value::Number(Number::Float(f))) => {
            let mut buf = Buffer::new();
            let mut s = format!("{letter}{}", buf.format(*f));
            s = s.replace('.', "-");
            s
        }
        _ => letter.to_string(),
    };
    Some(NormalizedVerb { raw })
}

fn normalize_param(word: &Word) -> Option<(String, &Value)> {
    let value = word.value.as_ref()?;
    let name = if let Some(name) = &word.name {
        name.clone()
    } else if let Some(letter) = word.letter {
        letter.to_string()
    } else {
        return None;
    };
    Some((name, value))
}

fn classify_value(value: &Value) -> Result<(ParamKind, ParamLiteral)> {
    Ok(match value {
        Value::Number(Number::Int(i)) => (ParamKind::Int, ParamLiteral::I64(*i)),
        Value::Number(Number::Float(f)) => (ParamKind::Float, ParamLiteral::F64(*f)),
        Value::Text(s) => (ParamKind::String, ParamLiteral::Str(s.clone())),
        Value::List(items) => classify_list(items)?,
    })
}

fn classify_list(items: &[Value]) -> Result<(ParamKind, ParamLiteral)> {
    if items.is_empty() {
        return Ok((ParamKind::ListString, ParamLiteral::ListStr(Vec::new())));
    }

    let mut saw_float = false;
    let mut saw_int = false;
    let mut saw_text = false;

    for item in items {
        match item {
            Value::Number(Number::Float(_)) => saw_float = true,
            Value::Number(Number::Int(_)) => saw_int = true,
            Value::Text(_) | Value::List(_) => saw_text = true,
        }
    }

    if saw_text {
        let mut vals = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Text(s) => vals.push(s.clone()),
                _ => bail!("mixed list types"),
            }
        }
        return Ok((ParamKind::ListString, ParamLiteral::ListStr(vals)));
    }

    if saw_float {
        let mut vals = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Number(Number::Float(f)) => vals.push(*f),
                Value::Number(Number::Int(i)) => vals.push(*i as f64),
                _ => bail!("mixed list types"),
            }
        }
        return Ok((ParamKind::ListFloat, ParamLiteral::ListF64(vals)));
    }

    if saw_int {
        let mut vals = Vec::with_capacity(items.len());
        for item in items {
            match item {
                Value::Number(Number::Int(i)) => vals.push(*i),
                _ => bail!("mixed list types"),
            }
        }
        return Ok((ParamKind::ListInt, ParamLiteral::ListI64(vals)));
    }

    bail!("unsupported list contents")
}

fn build_wit(verbs: &[VerbShape]) -> Result<String> {
    let mut pkg = Package::new(PackageName::new("job", "print", None));

    let mut world = World::new("job");

    for verb in verbs {
        let mut iface = Interface::new(verb.raw.to_kebab_case());
        let mut funcs = Vec::new();

        funcs.push(ResourceFunc::constructor());
        for (param, shape) in &verb.params {
            for kind in &shape.kinds {
                let mut func = ResourceFunc::method(
                    format!("set-{}{}", param.to_kebab_case(), kind_suffix(kind)),
                    false,
                );
                func.params_mut().item("value", type_for_kind(kind));
                funcs.push(func);
            }
        }
        funcs.push(ResourceFunc::method("submit", false));

        iface.type_def(TypeDef::resource("builder", funcs));
        world.named_interface_import(iface.name().clone());
        pkg.interface(iface);
    }

    world.function_export(StandaloneFunc::new("run", false));
    pkg.world(world);

    Ok(format!("{pkg}"))
}

fn type_for_kind(kind: &ParamKind) -> Type {
    match kind {
        ParamKind::Int => Type::S64,
        ParamKind::Float => Type::F64,
        ParamKind::String => Type::String,
        ParamKind::ListInt => Type::list(Type::S64),
        ParamKind::ListFloat => Type::list(Type::F64),
        ParamKind::ListString => Type::list(Type::String),
    }
}

fn kind_suffix(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::Int => "-int",
        ParamKind::Float => "-float",
        ParamKind::String => "-string",
        ParamKind::ListInt => "-list-int",
        ParamKind::ListFloat => "-list-float",
        ParamKind::ListString => "-list-string",
    }
}

fn literal_kind(lit: &ParamLiteral) -> ParamKind {
    match lit {
        ParamLiteral::I64(_) => ParamKind::Int,
        ParamLiteral::F64(_) => ParamKind::Float,
        ParamLiteral::Str(_) => ParamKind::String,
        ParamLiteral::ListI64(_) => ParamKind::ListInt,
        ParamLiteral::ListF64(_) => ParamKind::ListFloat,
        ParamLiteral::ListStr(_) => ParamKind::ListString,
    }
}

#[derive(Default)]
struct DataAllocator {
    offset: u32,
    segments: Vec<(u32, Vec<u8>)>,
}

impl DataAllocator {
    fn alloc(&mut self, mut bytes: Vec<u8>, align: u32) -> (u32, u32) {
        let align_mask = align.saturating_sub(1);
        let offset = (self.offset + align_mask) & !align_mask;
        let len = bytes.len() as u32;
        self.segments.push((offset, std::mem::take(&mut bytes)));
        self.offset = offset + len;
        (offset, len)
    }

    fn total_len(&self) -> u32 {
        self.offset
    }
}

fn build_wasm(verbs: &[VerbShape], stmts: &[CompiledStatement]) -> Result<Module> {
    let mut types = TypeSection::new();
    let mut type_cache: HashMap<(Vec<ValType>, Vec<ValType>), u32> = HashMap::new();
    let mut imports = ImportSection::new();
    let mut functions = FunctionSection::new();
    let mut exports = ExportSection::new();
    let mut code = CodeSection::new();
    let mut data = DataSection::new();

    let mut data_alloc = DataAllocator::default();

    let mut import_indices: HashMap<String, u32> = HashMap::new();

    let mut next_func_index = 0u32;

    let add_func_type =
        |params: Vec<ValType>,
         results: Vec<ValType>,
         types: &mut TypeSection,
         cache: &mut HashMap<(Vec<ValType>, Vec<ValType>), u32>| {
            if let Some(idx) = cache.get(&(params.clone(), results.clone())) {
                return *idx;
            }
            let idx = types.len();
            cache.insert((params.clone(), results.clone()), idx);
            types.ty().function(params, results);
            idx
        };

    for verb in verbs {
        let module = import_module_name(&verb.raw);
        let builder_ident = "builder".to_string();
        let builder_symbol = builder_ident.clone();
        let ctor_name = format!("[constructor]{builder_symbol}");

        // constructor -> builder handle (i32)
        let ty = add_func_type(vec![], vec![ValType::I32], &mut types, &mut type_cache);
        imports.import(&module, &ctor_name, EntityType::Function(ty));
        import_indices.insert(format!("{module}::{ctor_name}"), next_func_index);
        next_func_index += 1;

        // resource drop
        let drop_name = format!("[resource-drop]{builder_symbol}");
        let drop_ty = add_func_type(vec![ValType::I32], vec![], &mut types, &mut type_cache);
        imports.import(&module, &drop_name, EntityType::Function(drop_ty));
        import_indices.insert(format!("{module}::{drop_name}"), next_func_index);
        next_func_index += 1;

        for (param, shape) in &verb.params {
            for kind in &shape.kinds {
                let setter_name = format!(
                    "[method]{builder_symbol}.set-{}{}",
                    param.to_kebab_case(),
                    kind_suffix(kind)
                );
                let (params, results) = match kind {
                    ParamKind::Int => (vec![ValType::I32, ValType::I64], vec![]),
                    ParamKind::Float => (vec![ValType::I32, ValType::F64], vec![]),
                    ParamKind::String
                    | ParamKind::ListInt
                    | ParamKind::ListFloat
                    | ParamKind::ListString => {
                        (vec![ValType::I32, ValType::I32, ValType::I32], vec![])
                    }
                };
                let ty = add_func_type(params, results, &mut types, &mut type_cache);
                imports.import(&module, &setter_name, EntityType::Function(ty));
                import_indices.insert(format!("{module}::{setter_name}"), next_func_index);
                next_func_index += 1;
            }
        }

        let submit_name = format!("[method]{builder_symbol}.submit");
        let submit_ty = add_func_type(vec![ValType::I32], vec![], &mut types, &mut type_cache);
        imports.import(&module, &submit_name, EntityType::Function(submit_ty));
        import_indices.insert(format!("{module}::{submit_name}"), next_func_index);
        next_func_index += 1;
    }

    // run() function
    let run_type = add_func_type(vec![], vec![], &mut types, &mut type_cache);
    functions.function(run_type);
    let run_index = next_func_index;

    let mut func = Function::new(vec![(1, ValType::I32)]);

    for stmt in stmts {
        let module = import_module_name(&stmt.verb);
        // builder handle
        let builder_ident = "builder".to_string();
        let builder_symbol = builder_ident.clone();
        let ctor_name = format!("[constructor]{builder_symbol}");
        let lookup = format!("{module}::{ctor_name}");
        let ctor = *import_indices.get(&lookup).ok_or_else(|| {
            let keys: Vec<_> = import_indices.keys().cloned().collect();
            anyhow!("missing ctor key {lookup}; available: {keys:?}")
        })?;
        func.instruction(&Instruction::Call(ctor));
        func.instruction(&Instruction::LocalSet(0));

        for (param, literal) in &stmt.params {
            let kind = literal_kind(literal);
            let setter_name = format!(
                "[method]{builder_symbol}.set-{}{}",
                param.to_kebab_case(),
                kind_suffix(&kind)
            );
            let setter = *import_indices
                .get(&format!("{module}::{setter_name}"))
                .ok_or_else(|| anyhow!("missing setter for {module}:{param}"))?;

            func.instruction(&Instruction::LocalGet(0));
            emit_literal(&mut func, literal, &mut data_alloc);
            func.instruction(&Instruction::Call(setter));
        }
        let submit_name = format!("[method]{builder_symbol}.submit");
        let submit = *import_indices
            .get(&format!("{module}::{submit_name}"))
            .ok_or_else(|| anyhow!("missing submit for {module}"))?;
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::Call(submit));
    }

    func.instruction(&Instruction::End);
    code.function(&func);

    exports.export("run", ExportKind::Func, run_index);

    // Memory + data segments for strings/lists
    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&functions);

    let total = data_alloc.total_len();
    let pages = total.div_ceil(0x10000).max(1);
    let mem_type = MemoryType {
        minimum: pages as u64,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    };
    let mut memories = MemorySection::new();
    memories.memory(mem_type);
    module.section(&memories);

    exports.export("memory", ExportKind::Memory, 0);

    module.section(&exports);
    module.section(&code);
    if !data_alloc.segments.is_empty() {
        for (offset, bytes) in &data_alloc.segments {
            data.active(0, &ConstExpr::i32_const(*offset as i32), bytes.clone());
        }
        module.section(&data);
    }

    Ok(module)
}

fn build_component(wit: &str, core: &Module) -> Result<Vec<u8>> {
    let mut resolve = Resolve::default();
    let pkg = resolve.push_str("job.wit", wit)?;
    // World name matches what build_wit emits.
    let world = resolve.select_world(&[pkg], Some("job"))?;

    // Start from core bytes and embed WIT metadata so the encoder can lift it.
    let mut core_bytes = core.clone().finish();
    embed_component_metadata(&mut core_bytes, &resolve, world, StringEncoding::UTF8)?;

    let component = ComponentEncoder::default()
        .module(&core_bytes)?
        .validate(true)
        .encode()?;

    Ok(component)
}

fn emit_literal(func: &mut Function, lit: &ParamLiteral, data: &mut DataAllocator) {
    match lit {
        ParamLiteral::I64(i) => {
            func.instruction(&Instruction::I64Const(*i));
        }
        ParamLiteral::F64(f) => {
            func.instruction(&Instruction::F64Const(Ieee64::from(*f)));
        }
        ParamLiteral::Str(s) => {
            let (offset, len) = data.alloc(s.as_bytes().to_vec(), 1);
            func.instruction(&Instruction::I32Const(offset as i32));
            func.instruction(&Instruction::I32Const(len as i32));
        }
        ParamLiteral::ListI64(items) => {
            let mut bytes = Vec::with_capacity(items.len() * 8);
            for i in items {
                bytes.extend_from_slice(&i.to_le_bytes());
            }
            let (offset, len) = data.alloc(bytes, 8);
            func.instruction(&Instruction::I32Const(offset as i32));
            func.instruction(&Instruction::I32Const((len / 8) as i32));
        }
        ParamLiteral::ListF64(items) => {
            let mut bytes = Vec::with_capacity(items.len() * 8);
            for f in items {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            let (offset, len) = data.alloc(bytes, 8);
            func.instruction(&Instruction::I32Const(offset as i32));
            func.instruction(&Instruction::I32Const((len / 8) as i32));
        }
        ParamLiteral::ListStr(items) => {
            let mut string_spans: Vec<(u32, u32)> = Vec::with_capacity(items.len());
            for s in items {
                let (offset, len) = data.alloc(s.as_bytes().to_vec(), 1);
                string_spans.push((offset, len));
            }

            let mut bytes = Vec::with_capacity(items.len() * 8);
            for (offset, len) in &string_spans {
                bytes.extend_from_slice(&offset.to_le_bytes());
                bytes.extend_from_slice(&len.to_le_bytes());
            }
            let (offset, len) = data.alloc(bytes, 4);
            func.instruction(&Instruction::I32Const(offset as i32));
            func.instruction(&Instruction::I32Const((len / 8) as i32));
        }
    }
}

fn import_module_name(raw: &str) -> String {
    format!("job:print/{}", raw.to_kebab_case())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::Parser;

    #[test]
    fn builds_wit_and_wasm() {
        let input = "G1 X1.5 Y2 Z3\nM104 S200\nG1 X4.0 Y5.5\n";
        let out = compile_gcode(input).expect("compile");

        assert!(out.wit.contains("interface g1"));
        assert!(out.wit.contains("resource builder"));
        assert!(out.wit.contains("constructor();"));
        assert!(out.wit.contains("set-x-float: func"));
        assert!(out.wit.contains("export run"));
        assert!(!out.wasm.is_empty());
        assert!(!out.component.is_empty());
        // core header magic
        assert_eq!(&out.wasm[..4], b"\0asm");
        // component header magic
        assert_eq!(&out.component[..4], b"\0asm");
        // component should parse via wasmparser
        assert!(Parser::is_component(&out.component));
    }

    #[test]
    fn preserves_float_verb_with_hyphen() {
        let input = "G1.0 X1\n";
        let out = compile_gcode(input).expect("compile");
        assert!(out.wit.contains("interface g1-0"));
    }
}
