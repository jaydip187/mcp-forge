//! GraphQL schemas (introspection JSON or SDL) → one tool per root query
//! and mutation field, with the field's arguments as typed variables and a
//! generated selection of the result (scalars, then nested objects a few
//! levels deep, capped so documents stay small).

use std::collections::HashMap;

use forge_core::{
    GraphqlOperation, GraphqlSpec, GraphqlVar, HttpMethod, ToolInput, ToolKind, DEFAULT_TIMEOUT_MS,
};
use serde_json::{json, Map, Value};

use crate::openapi3::{BuiltOperation, Imported, Options, SpecInfo};
use crate::ImportError;

/// Nested objects are selected this many levels deep (`edges { node { … } }` needs 3).
const MAX_DEPTH: usize = 3;
/// Fields selected per object, and in all.
const MAX_FIELDS_PER_OBJECT: usize = 25;
const MAX_FIELDS: usize = 60;
const MAX_UNION_MEMBERS: usize = 8;
const MAX_INPUT_DEPTH: usize = 4;

#[derive(Debug, Clone, PartialEq)]
enum TypeRef {
    Named(String),
    List(Box<TypeRef>),
    NonNull(Box<TypeRef>),
}

impl TypeRef {
    fn base(&self) -> &str {
        match self {
            TypeRef::Named(n) => n,
            TypeRef::List(t) | TypeRef::NonNull(t) => t.base(),
        }
    }

    fn non_null(&self) -> bool {
        matches!(self, TypeRef::NonNull(_))
    }

    /// As written in GraphQL: `[ID!]!`.
    fn render(&self) -> String {
        match self {
            TypeRef::Named(n) => n.clone(),
            TypeRef::List(t) => format!("[{}]", t.render()),
            TypeRef::NonNull(t) => format!("{}!", t.render()),
        }
    }
}

#[derive(Debug, Clone)]
struct InputValue {
    name: String,
    description: Option<String>,
    ty: TypeRef,
    has_default: bool,
}

#[derive(Debug, Clone)]
struct Field {
    name: String,
    description: Option<String>,
    args: Vec<InputValue>,
    ty: TypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scalar,
    Object,
    Interface,
    Union,
    Enum,
    InputObject,
}

#[derive(Debug, Clone)]
struct TypeDef {
    kind: Kind,
    description: Option<String>,
    fields: Vec<Field>,
    input_fields: Vec<InputValue>,
    enum_values: Vec<String>,
    possible: Vec<String>,
}

#[derive(Debug, Default)]
struct Schema {
    query: Option<String>,
    mutation: Option<String>,
    types: HashMap<String, TypeDef>,
}

/// Whether `doc` is an introspection result (`{data: {__schema}}` or `{__schema}`).
pub fn is_introspection(doc: &Value) -> bool {
    doc.pointer("/data/__schema").is_some() || doc.get("__schema").is_some()
}

/// Whether `text` looks like GraphQL SDL.
pub fn is_sdl(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        return false;
    }
    let has_def = text.lines().any(|l| {
        let l = l.trim_start();
        [
            "type ",
            "schema ",
            "schema{",
            "extend ",
            "input ",
            "interface ",
            "enum ",
            "union ",
            "scalar ",
            "directive ",
        ]
        .iter()
        .any(|k| l.starts_with(k))
    });
    has_def && (text.contains("Query") || text.contains("Mutation") || text.contains("schema"))
}

// ---- reading schemas ---------------------------------------------------------

fn type_ref(v: &Value) -> Option<TypeRef> {
    let of = || v.get("ofType").and_then(type_ref);
    match v.get("kind")?.as_str()? {
        "NON_NULL" => Some(TypeRef::NonNull(Box::new(of()?))),
        "LIST" => Some(TypeRef::List(Box::new(of()?))),
        _ => Some(TypeRef::Named(v.get("name")?.as_str()?.to_owned())),
    }
}

fn input_value(v: &Value) -> Option<InputValue> {
    Some(InputValue {
        name: v.get("name")?.as_str()?.to_owned(),
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ty: type_ref(v.get("type")?)?,
        has_default: v.get("defaultValue").is_some_and(|d| !d.is_null()),
    })
}

fn from_introspection(doc: &Value) -> Result<Schema, ImportError> {
    let schema = doc
        .pointer("/data/__schema")
        .or_else(|| doc.get("__schema"))
        .ok_or_else(|| {
            ImportError::Unsupported("No `__schema` in the introspection result.".into())
        })?;
    let name_at = |p: &str| schema.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let mut s = Schema {
        query: name_at("/queryType/name"),
        mutation: name_at("/mutationType/name"),
        ..Schema::default()
    };
    let list = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    for t in schema
        .get("types")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = t.get("name").and_then(Value::as_str) else {
            continue;
        };
        let kind = match t.get("kind").and_then(Value::as_str).unwrap_or("") {
            "OBJECT" => Kind::Object,
            "INTERFACE" => Kind::Interface,
            "UNION" => Kind::Union,
            "ENUM" => Kind::Enum,
            "INPUT_OBJECT" => Kind::InputObject,
            _ => Kind::Scalar,
        };
        let fields = list(t, "fields")
            .iter()
            .filter_map(|f| {
                Some(Field {
                    name: f.get("name")?.as_str()?.to_owned(),
                    description: f
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    args: list(f, "args").iter().filter_map(input_value).collect(),
                    ty: type_ref(f.get("type")?)?,
                })
            })
            .collect();
        s.types.insert(
            name.to_owned(),
            TypeDef {
                kind,
                description: t
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                fields,
                input_fields: list(t, "inputFields")
                    .iter()
                    .filter_map(input_value)
                    .collect(),
                enum_values: list(t, "enumValues")
                    .iter()
                    .filter_map(|e| e.get("name")?.as_str().map(str::to_owned))
                    .collect(),
                possible: list(t, "possibleTypes")
                    .iter()
                    .filter_map(|e| e.get("name")?.as_str().map(str::to_owned))
                    .collect(),
            },
        );
    }
    Ok(s)
}

fn from_sdl(text: &str) -> Result<Schema, ImportError> {
    use graphql_parser::schema::{Definition, Type, TypeDefinition, TypeExtension};
    let doc = graphql_parser::parse_schema::<String>(text)
        .map_err(|e| ImportError::Parse(format!("Couldn't read the GraphQL schema: {e}")))?;
    fn tref(t: &Type<String>) -> TypeRef {
        match t {
            Type::NamedType(n) => TypeRef::Named(n.clone()),
            Type::ListType(i) => TypeRef::List(Box::new(tref(i))),
            Type::NonNullType(i) => TypeRef::NonNull(Box::new(tref(i))),
        }
    }
    let deprecated =
        |d: &[graphql_parser::schema::Directive<String>]| d.iter().any(|x| x.name == "deprecated");
    let input = |v: &graphql_parser::schema::InputValue<String>| InputValue {
        name: v.name.clone(),
        description: v.description.clone(),
        ty: tref(&v.value_type),
        has_default: v.default_value.is_some(),
    };
    let field = |f: &graphql_parser::schema::Field<String>| Field {
        name: f.name.clone(),
        description: f.description.clone(),
        args: f.arguments.iter().map(input).collect(),
        ty: tref(&f.field_type),
    };
    let mut s = Schema::default();
    let mut add =
        |name: &str, def: TypeDef, extend: bool| match s.types.get_mut(name).filter(|_| extend) {
            Some(existing) => {
                existing.fields.extend(def.fields);
                existing.input_fields.extend(def.input_fields);
                existing.enum_values.extend(def.enum_values);
                existing.possible.extend(def.possible);
            }
            None => {
                s.types.insert(name.to_owned(), def);
            }
        };
    let blank = |kind| TypeDef {
        kind,
        description: None,
        fields: vec![],
        input_fields: vec![],
        enum_values: vec![],
        possible: vec![],
    };
    let mut schema_def = None;
    for d in &doc.definitions {
        let (t, extend) = match d {
            Definition::SchemaDefinition(sd) => {
                schema_def = Some((sd.query.clone(), sd.mutation.clone()));
                continue;
            }
            Definition::TypeDefinition(t) => (t.clone(), false),
            Definition::TypeExtension(TypeExtension::Object(o)) => (
                TypeDefinition::Object(graphql_parser::schema::ObjectType {
                    position: o.position,
                    description: None,
                    name: o.name.clone(),
                    implements_interfaces: o.implements_interfaces.clone(),
                    directives: o.directives.clone(),
                    fields: o.fields.clone(),
                }),
                true,
            ),
            _ => continue,
        };
        match &t {
            TypeDefinition::Object(o) => add(
                &o.name,
                TypeDef {
                    description: o.description.clone(),
                    fields: o
                        .fields
                        .iter()
                        .filter(|f| !deprecated(&f.directives))
                        .map(field)
                        .collect(),
                    ..blank(Kind::Object)
                },
                extend,
            ),
            TypeDefinition::Interface(i) => add(
                &i.name,
                TypeDef {
                    description: i.description.clone(),
                    fields: i
                        .fields
                        .iter()
                        .filter(|f| !deprecated(&f.directives))
                        .map(field)
                        .collect(),
                    ..blank(Kind::Interface)
                },
                extend,
            ),
            TypeDefinition::Union(u) => add(
                &u.name,
                TypeDef {
                    description: u.description.clone(),
                    possible: u.types.clone(),
                    ..blank(Kind::Union)
                },
                extend,
            ),
            TypeDefinition::Enum(e) => add(
                &e.name,
                TypeDef {
                    description: e.description.clone(),
                    enum_values: e.values.iter().map(|v| v.name.clone()).collect(),
                    ..blank(Kind::Enum)
                },
                extend,
            ),
            TypeDefinition::InputObject(i) => add(
                &i.name,
                TypeDef {
                    description: i.description.clone(),
                    input_fields: i.fields.iter().map(input).collect(),
                    ..blank(Kind::InputObject)
                },
                extend,
            ),
            TypeDefinition::Scalar(sc) => add(
                &sc.name,
                TypeDef {
                    description: sc.description.clone(),
                    ..blank(Kind::Scalar)
                },
                extend,
            ),
        }
    }
    let (q, m) = schema_def.unwrap_or((None, None));
    s.query = q.or_else(|| s.types.contains_key("Query").then(|| "Query".into()));
    s.mutation = m.or_else(|| s.types.contains_key("Mutation").then(|| "Mutation".into()));
    Ok(s)
}

// ---- building tools ----------------------------------------------------------

fn kind_of(s: &Schema, name: &str) -> Kind {
    match name {
        "Int" | "Float" | "String" | "Boolean" | "ID" => Kind::Scalar,
        _ => s.types.get(name).map_or(Kind::Scalar, |t| t.kind),
    }
}

/// JSON Schema for an input type (variables and input objects).
fn input_schema(s: &Schema, t: &TypeRef, depth: usize) -> Value {
    match t {
        TypeRef::NonNull(inner) => input_schema(s, inner, depth),
        TypeRef::List(inner) => json!({ "type": "array", "items": input_schema(s, inner, depth) }),
        TypeRef::Named(n) => match n.as_str() {
            "Int" => json!({ "type": "integer" }),
            "Float" => json!({ "type": "number" }),
            "String" | "ID" => json!({ "type": "string" }),
            "Boolean" => json!({ "type": "boolean" }),
            _ => match s.types.get(n) {
                Some(td) if td.kind == Kind::Enum => {
                    json!({ "type": "string", "enum": td.enum_values })
                }
                Some(td) if td.kind == Kind::InputObject && depth < MAX_INPUT_DEPTH => {
                    let mut props = Map::new();
                    let mut required = vec![];
                    for f in &td.input_fields {
                        let mut fs = input_schema(s, &f.ty, depth + 1);
                        if let (Some(d), Some(o)) = (&f.description, fs.as_object_mut()) {
                            o.insert("description".into(), json!(d));
                        }
                        props.insert(f.name.clone(), fs);
                        if f.ty.non_null() && !f.has_default {
                            required.push(f.name.clone());
                        }
                    }
                    json!({ "type": "object", "properties": props, "required": required })
                }
                Some(td) if td.kind == Kind::InputObject => json!({ "type": "object" }),
                _ => json!({ "description": format!("GraphQL scalar {n}") }),
            },
        },
    }
}

/// The selection for a value of type `name`, or `None` for leaves.
fn selection(
    s: &Schema,
    name: &str,
    depth: usize,
    budget: &mut usize,
    indent: usize,
) -> Option<String> {
    let pad = "  ".repeat(indent);
    match kind_of(s, name) {
        Kind::Scalar | Kind::Enum | Kind::InputObject => None,
        Kind::Union => {
            let td = s.types.get(name)?;
            let mut lines = vec![format!("{pad}__typename")];
            for member in td.possible.iter().take(MAX_UNION_MEMBERS) {
                let inner = leaves(s, member, budget, indent + 1);
                if !inner.is_empty() {
                    lines.push(format!(
                        "{pad}... on {member} {{\n{}\n{pad}}}",
                        inner.join("\n")
                    ));
                }
            }
            Some(lines.join("\n"))
        }
        Kind::Object | Kind::Interface => {
            let td = s.types.get(name)?;
            let mut lines = vec![];
            for f in &td.fields {
                if lines.len() >= MAX_FIELDS_PER_OBJECT || *budget == 0 {
                    break;
                }
                // A field needing arguments can't be selected without values.
                if f.args.iter().any(|a| a.ty.non_null() && !a.has_default) {
                    continue;
                }
                match kind_of(s, f.ty.base()) {
                    Kind::Scalar | Kind::Enum => {
                        lines.push(format!("{pad}{}", f.name));
                        *budget -= 1;
                    }
                    _ if depth < MAX_DEPTH => {
                        *budget -= 1;
                        if let Some(inner) =
                            selection(s, f.ty.base(), depth + 1, budget, indent + 1)
                        {
                            lines.push(format!("{pad}{} {{\n{inner}\n{pad}}}", f.name));
                        } else {
                            *budget += 1;
                        }
                    }
                    _ => {}
                }
            }
            if lines.is_empty() {
                lines.push(format!("{pad}__typename"));
            }
            Some(lines.join("\n"))
        }
    }
}

/// Scalar and enum fields of an object (for union members).
fn leaves(s: &Schema, name: &str, budget: &mut usize, indent: usize) -> Vec<String> {
    let pad = "  ".repeat(indent);
    let Some(td) = s.types.get(name) else {
        return vec![];
    };
    td.fields
        .iter()
        .filter(|f| f.args.iter().all(|a| !a.ty.non_null() || a.has_default))
        .filter(|f| matches!(kind_of(s, f.ty.base()), Kind::Scalar | Kind::Enum))
        .take(MAX_FIELDS_PER_OBJECT.min(*budget))
        .map(|f| {
            *budget = budget.saturating_sub(1);
            format!("{pad}{}", f.name)
        })
        .collect()
}

fn pascal(name: &str) -> String {
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn build(s: &Schema, op: GraphqlOperation, f: &Field, endpoint: Option<&str>) -> BuiltOperation {
    let op_name = pascal(&f.name);
    let defs: Vec<String> = f
        .args
        .iter()
        .map(|a| format!("${}: {}", a.name, a.ty.render()))
        .collect();
    let call_args: Vec<String> = f
        .args
        .iter()
        .map(|a| format!("{0}: ${0}", a.name))
        .collect();
    let header = if defs.is_empty() {
        format!("{} {op_name}", op.as_str())
    } else {
        format!("{} {op_name}({})", op.as_str(), defs.join(", "))
    };
    let call = if call_args.is_empty() {
        f.name.clone()
    } else {
        format!("{}({})", f.name, call_args.join(", "))
    };
    let mut budget = MAX_FIELDS;
    let document = match selection(s, f.ty.base(), 1, &mut budget, 2) {
        Some(sel) => format!("{header} {{\n  {call} {{\n{sel}\n  }}\n}}"),
        None => format!("{header} {{\n  {call}\n}}"),
    };
    let variables = f
        .args
        .iter()
        .map(|a| {
            // Optional with a server-side default: keep it optional for the agent.
            let mut t = a.ty.render();
            if a.has_default {
                t = t.trim_end_matches('!').to_owned();
            }
            GraphqlVar {
                name: a.name.clone(),
                graphql_type: t,
                description: a.description.clone().unwrap_or_default(),
                schema: Some(input_schema(s, &a.ty, 0)),
            }
        })
        .collect();
    let mut description = f.description.clone().unwrap_or_default().trim().to_owned();
    let returns = format!("Returns {}.", f.ty.render());
    if let Some(td) = s
        .types
        .get(f.ty.base())
        .and_then(|t| t.description.as_deref())
    {
        let first = td.lines().next().unwrap_or("").trim();
        description = if description.is_empty() {
            format!("{returns} {first}")
        } else {
            format!("{description}\n\n{returns}")
        };
    } else if description.is_empty() {
        description = returns;
    }
    let key = format!("{} {}", op.as_str(), f.name);
    let (supported, warnings) = match endpoint {
        Some(_) => (true, vec![]),
        None => (
            false,
            vec!["Enter the API's GraphQL endpoint above to import this.".to_owned()],
        ),
    };
    BuiltOperation {
        key: key.clone(),
        operation_id: Some(key.clone()),
        method: HttpMethod::Post,
        path: key,
        tags: vec![pascal(op.as_str())],
        summary: f
            .description
            .clone()
            .map(|d| d.lines().next().unwrap_or("").to_owned()),
        deprecated: false,
        needs_auth: false,
        tool: ToolInput {
            name: crate::sanitize_name(&f.name, false),
            description,
            kind: ToolKind::Graphql(GraphqlSpec {
                endpoint: endpoint.unwrap_or("").to_owned(),
                operation: op,
                document,
                operation_name: Some(op_name),
                variables,
                auth_ref: None,
                timeout_ms: DEFAULT_TIMEOUT_MS,
            }),
            enabled: true,
            tags: vec![pascal(op.as_str())],
            response_shape: None,
            // Like other imports, approval starts off; the editor can turn it on.
            require_approval: false,
        },
        supported,
        warnings,
    }
}

fn import_schema(s: &Schema, opts: &Options) -> Result<Imported, ImportError> {
    let endpoint = opts
        .base_url
        .as_deref()
        .or(opts.source_url.as_deref())
        .map(str::trim)
        .filter(|e| !e.is_empty());
    let mut operations = vec![];
    for (op, root) in [
        (GraphqlOperation::Query, s.query.as_deref()),
        (GraphqlOperation::Mutation, s.mutation.as_deref()),
    ] {
        let Some(td) = root.and_then(|r| s.types.get(r)) else {
            continue;
        };
        for f in &td.fields {
            operations.push(build(s, op, f, endpoint));
        }
    }
    if operations.is_empty() {
        return Err(ImportError::Unsupported(
            "The GraphQL schema has no query or mutation fields.".into(),
        ));
    }
    let title = endpoint
        .and_then(|e| url::Url::parse(e).ok())
        .and_then(|u| u.host_str().map(|h| format!("GraphQL API at {h}")))
        .unwrap_or_else(|| "GraphQL API".into());
    Ok(Imported {
        info: SpecInfo {
            title,
            api_version: String::new(),
            openapi: String::new(),
            kind: "graphql".into(),
            servers: endpoint.map(|e| vec![e.to_owned()]).unwrap_or_default(),
            base_url: endpoint.map(str::to_owned),
            security_schemes: vec![],
        },
        operations,
    })
}

pub fn import_introspection(doc: &Value, opts: &Options) -> Result<Imported, ImportError> {
    import_schema(&from_introspection(doc)?, opts)
}

pub fn import_sdl(text: &str, opts: &Options) -> Result<Imported, ImportError> {
    import_schema(&from_sdl(text)?, opts)
}
