//! Native implementations for `clojure.edn`.
//!
//! EDN shares the source reader's lexer and parser, then applies a strict data
//! conversion pass. The conversion rejects Clojure-only reader forms, checks
//! collection uniqueness, and resolves EDN tagged literals through the
//! caller's `:readers` and `:default` options.

use std::sync::Arc;

use chrono::{DateTime, NaiveDate};
use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind, Parser};
use cljrs_runtime::builtins::form::{attach_meta, form_to_value};
use cljrs_runtime::env::callback;
use cljrs_value::{
    Arity, Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, ResourceHandle,
    SetValue, Symbol, TypeInstance, Value, ValueError, ValueResult,
};

use crate::io::{IoReader, StringReader};
use crate::register_fns;

pub fn register(globals: &Arc<cljrs_runtime::env::env::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("read-string", Arity::Variadic { min: 1 }, edn_read_string),
            ("read", Arity::Variadic { min: 1 }, edn_read),
        ]
    );
}

fn option(opts: Option<&Value>, name: &str) -> Option<Value> {
    let Value::Map(map) = opts?.unwrap_meta() else {
        return None;
    };
    map.get(&Value::keyword(Keyword::simple(name)))
}

/// Read one EDN form from a string.
///
/// `(clojure.edn/read-string s)`
/// `(clojure.edn/read-string opts s)`
fn edn_read_string(args: &[Value]) -> ValueResult<Value> {
    let (opts, s) = match args.len() {
        1 => (None, &args[0]),
        2 => (Some(&args[0]), &args[1]),
        n => {
            return Err(ValueError::ArityError {
                name: "clojure.edn/read-string".into(),
                expected: "1-2".into(),
                got: n,
            });
        }
    };

    let src = match s.unwrap_meta() {
        Value::Str(s) => s.get().clone(),
        Value::Nil if opts.is_none() => return Ok(Value::Nil),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };

    read_edn_source(src, opts)
}

/// Read one EDN form from a reader resource.
///
/// `(clojure.edn/read reader)`
/// `(clojure.edn/read opts reader)`
fn edn_read(args: &[Value]) -> ValueResult<Value> {
    let (opts, reader_val) = match args.len() {
        1 => (None, &args[0]),
        2 => (Some(&args[0]), &args[1]),
        n => {
            return Err(ValueError::ArityError {
                name: "clojure.edn/read".into(),
                expected: "1-2".into(),
                got: n,
            });
        }
    };

    let src = match reader_val.unwrap_meta() {
        Value::Resource(r) => read_all_from_resource(r)?,
        v => {
            return Err(ValueError::WrongType {
                expected: "reader",
                got: v.type_name().to_string(),
            });
        }
    };

    read_edn_source(src, opts)
}

fn read_edn_source(src: String, opts: Option<&Value>) -> ValueResult<Value> {
    if src
        .trim_start_matches(|c: char| c.is_whitespace() || c == ',')
        .starts_with("#!")
    {
        return Err(edn_error("shebangs are not valid EDN"));
    }

    let mut parser = Parser::new(src.clone(), "<edn>".to_string());
    let parsed = parser
        .parse_one()
        .map_err(|e| edn_error(format!("EDN parse error: {e}")))?;

    // Discard is syntactic, but tagged literals still go through their normal
    // validation and reader functions before their resulting value is ignored.
    for discarded in parser.take_discarded_forms() {
        strict_form_to_value(&discarded, opts)?;
    }

    let Some(form) = parsed else {
        return match option(opts, "eof") {
            Some(v) => Ok(v),
            None if opts.is_none() => Ok(Value::Nil),
            None => Err(edn_error("EOF while reading EDN")),
        };
    };

    if let Some(next) = src
        .get(form.span.end..)
        .and_then(|tail| tail.chars().next())
        && !is_token_delimiter(next)
    {
        return Err(edn_error("invalid character after EDN token"));
    }

    strict_form_to_value(&form, opts)
}

fn is_token_delimiter(c: char) -> bool {
    c.is_whitespace()
        || c == ','
        || matches!(
            c,
            '(' | ')' | '[' | ']' | '{' | '}' | '"' | ';' | '#' | '\'' | '@' | '^' | '~'
        )
}

fn strict_form_to_value(form: &Form, opts: Option<&Value>) -> ValueResult<Value> {
    match &form.kind {
        FormKind::Nil
        | FormKind::Bool(_)
        | FormKind::Int(_)
        | FormKind::BigInt(_)
        | FormKind::Float(_)
        | FormKind::BigDecimal(_)
        | FormKind::Char(_)
        | FormKind::Str(_)
        | FormKind::Symbolic(_) => literal_value(form),

        FormKind::Ratio(_) => {
            let value = literal_value(form)?;
            if matches!(value, Value::Nil) {
                Err(edn_error("ratio denominator cannot be zero"))
            } else {
                Ok(value)
            }
        }

        FormKind::Symbol(name) => {
            validate_named(name, "symbol")?;
            Ok(Value::symbol(Symbol::parse(name)))
        }
        FormKind::Keyword(name) => {
            validate_named(name, "keyword")?;
            Ok(Value::keyword(Keyword::parse(name)))
        }

        FormKind::List(forms) => Ok(Value::List(GcPtr::new(PersistentList::from_iter(
            strict_forms(forms, opts)?,
        )))),
        FormKind::Vector(forms) => Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
            strict_forms(forms, opts)?,
        )))),
        FormKind::Map(forms) => strict_map(forms, opts),
        FormKind::Set(forms) => strict_set(forms, opts),

        FormKind::Meta(meta, inner) => {
            if !supports_edn_metadata(inner) {
                return Err(edn_error("metadata target does not support metadata"));
            }
            let annotation = strict_meta(meta, opts)?;
            Ok(attach_meta(strict_form_to_value(inner, opts)?, annotation))
        }
        FormKind::TaggedLiteral(tag, inner) => strict_tagged_literal(tag, inner, opts),

        FormKind::Regex(_)
        | FormKind::AutoKeyword(_)
        | FormKind::AutoSymbol(_)
        | FormKind::Quote(_)
        | FormKind::SyntaxQuote(_)
        | FormKind::Unquote(_)
        | FormKind::UnquoteSplice(_)
        | FormKind::Deref(_)
        | FormKind::Var(_)
        | FormKind::AnonFn(_)
        | FormKind::ReaderCond { .. } => Err(edn_error("form is not valid EDN")),
    }
}

fn literal_value(form: &Form) -> ValueResult<Value> {
    form_to_value(form).map_err(|e| edn_error(e.to_string()))
}

fn strict_forms(forms: &[Form], opts: Option<&Value>) -> ValueResult<Vec<Value>> {
    forms
        .iter()
        .map(|form| strict_form_to_value(form, opts))
        .collect()
}

fn strict_map(forms: &[Form], opts: Option<&Value>) -> ValueResult<Value> {
    if !forms.len().is_multiple_of(2) {
        return Err(edn_error(
            "map literal must contain an even number of forms",
        ));
    }
    let mut map = MapValue::empty();
    for pair in forms.chunks_exact(2) {
        let key = strict_form_to_value(&pair[0], opts)?;
        if map.contains_key(&key) {
            return Err(edn_error("duplicate map key"));
        }
        map = map.assoc(key, strict_form_to_value(&pair[1], opts)?);
    }
    Ok(Value::Map(map))
}

fn strict_set(forms: &[Form], opts: Option<&Value>) -> ValueResult<Value> {
    let mut set = PersistentHashSet::empty();
    for form in forms {
        let value = strict_form_to_value(form, opts)?;
        if set.contains(&value) {
            return Err(edn_error("duplicate set element"));
        }
        set.conj_mut(value);
    }
    Ok(Value::Set(SetValue::Hash(GcPtr::new(set))))
}

fn strict_meta(meta: &Form, opts: Option<&Value>) -> ValueResult<Value> {
    let tag_key = Value::keyword(Keyword::simple("tag"));
    match &meta.kind {
        FormKind::Keyword(_) => Ok(Value::Map(
            MapValue::empty().assoc(strict_form_to_value(meta, opts)?, Value::Bool(true)),
        )),
        FormKind::Symbol(_) | FormKind::Str(_) => Ok(Value::Map(
            MapValue::empty().assoc(tag_key, strict_form_to_value(meta, opts)?),
        )),
        FormKind::Map(_) => strict_form_to_value(meta, opts),
        _ => Err(edn_error("invalid metadata annotation")),
    }
}

fn strict_tagged_literal(tag: &str, inner: &Form, opts: Option<&Value>) -> ValueResult<Value> {
    validate_named(tag, "tag")?;
    let tag_value = Value::symbol(Symbol::parse(tag));
    let value = strict_form_to_value(inner, opts)?;

    if let Some(Value::Map(readers)) = option(opts, "readers").as_ref().map(Value::unwrap_meta)
        && let Some(reader) = readers.get(&tag_value)
    {
        return callback::invoke(&reader, vec![value]);
    }

    match tag {
        "uuid" => match value.unwrap_meta() {
            Value::Str(s) => uuid::Uuid::parse_str(s.get())
                .map(|uuid| Value::uuid(uuid.as_u128()))
                .map_err(|_| edn_error("invalid UUID literal")),
            _ => Err(edn_error("#uuid requires a string")),
        },
        "inst" => match value.unwrap_meta() {
            Value::Str(s) => instant_value(s.get()),
            _ => Err(edn_error("#inst requires a string")),
        },
        _ => match option(opts, "default") {
            Some(default) => callback::invoke(&default, vec![tag_value, value]),
            None => Err(edn_error(format!("no reader function for tag {tag}"))),
        },
    }
}

fn instant_value(text: &str) -> ValueResult<Value> {
    let millis = if let Ok(date_time) = DateTime::parse_from_rfc3339(text) {
        date_time.timestamp_millis()
    } else if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        date.and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc()
            .timestamp_millis()
    } else {
        return Err(edn_error("invalid instant literal"));
    };

    let fields = MapValue::empty().assoc(
        Value::keyword(Keyword::simple("millis")),
        Value::Long(millis),
    );
    Ok(Value::TypeInstance(GcPtr::new(TypeInstance {
        type_tag: Arc::from("java.util.Date"),
        fields,
        mutable: None,
    })))
}

fn validate_named(name: &str, kind: &str) -> ValueResult<()> {
    if name == "/" {
        return Ok(());
    }
    if name.is_empty() || name.starts_with('/') || (name.ends_with('/') && !name.ends_with("//")) {
        return Err(edn_error(format!("invalid {kind}")));
    }
    if name != "/"
        && let Some((namespace, local)) = name.split_once('/')
        && (namespace.is_empty() || local.is_empty() || (local != "/" && local.contains('/')))
    {
        return Err(edn_error(format!("invalid {kind}")));
    }
    Ok(())
}

fn supports_edn_metadata(form: &Form) -> bool {
    match &form.kind {
        FormKind::Symbol(_)
        | FormKind::List(_)
        | FormKind::Vector(_)
        | FormKind::Map(_)
        | FormKind::Set(_) => true,
        FormKind::Meta(_, inner) => supports_edn_metadata(inner),
        _ => false,
    }
}

fn edn_error(message: impl Into<String>) -> ValueError {
    ValueError::Other(message.into())
}

fn read_all_from_resource(r: &ResourceHandle) -> ValueResult<String> {
    if let Some(reader) = r.downcast::<IoReader>() {
        reader.read_all()
    } else if let Some(reader) = r.downcast::<StringReader>() {
        reader.read_all()
    } else {
        Err(ValueError::Other("not a readable resource".into()))
    }
}
