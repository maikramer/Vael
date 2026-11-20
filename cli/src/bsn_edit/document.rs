//! Index the physical document. Jackdaw parses all type paths, component
//! bodies and values. Keep token end offsets separately so replacements leave
//! trailing comments and whitespace intact.
use anyhow::{Context, Result, bail, ensure};
use jackdaw_bsn::syntax::{
    lex::Token,
    parse_stream::{Parse, ParseStream},
    span::Span,
    types::{
        At, BsnEntry, BsnFields, BsnValue, Colon, Comma, DoubleMinus, Hash, Ident, LBrace,
        LBracket, Path as TypePath, RBrace, RBracket, StringLit,
    },
};
use serde::Serialize;
use std::ops::Range;

const WORLD_NODE: &str = "bevy_world_xml::bsn::WorldNode";
const SCENE_ID: &str = "jackdaw_scene_types::node_id::SceneNodeId";

#[derive(Debug, Serialize)]
pub struct Node {
    pub path: String,
    pub name: Option<String>,
    pub tag: Option<String>,
    // u64 IDs must survive clients which parse JSON numbers as doubles.
    pub id: Option<String>,
    pub line: u32,
    pub components: Vec<String>,
    #[serde(skip)]
    bodies: Vec<Range<usize>>,
    #[serde(skip)]
    children: Vec<(usize, bool)>,
    #[serde(skip)]
    pub range: Range<usize>,
}

struct Cursor<'a> {
    input: ParseStream<'a>,
    ends: Vec<usize>,
    text: &'a str,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str) -> Result<Self> {
        let mut ends = Vec::new();
        // Bound recursion before passing nested values/generics to the parser.
        let mut depth = 0usize;
        for token in Span::from(text) {
            let t = token?;
            match t.token {
                Token::LParen | Token::LBracket | Token::LBrace | Token::Less => {
                    depth += 1;
                    ensure!(
                        depth <= 256,
                        "documento demasiado profundo na linha {}",
                        t.span.line
                    );
                }
                Token::RParen | Token::RBracket | Token::RBrace | Token::Greater => {
                    depth = depth.saturating_sub(1)
                }
                _ => {}
            }
            // Jackdaw spans a string's escaped contents, excluding the quotes.
            ends.push(
                t.span.offset
                    + t.span.fragment.len()
                    + usize::from(matches!(t.token, Token::StringLit(_))),
            );
        }
        Ok(Self {
            input: Span::from(text).into(),
            ends,
            text,
        })
    }

    fn start(&mut self) -> usize {
        self.input.peek_token().map_or(self.text.len(), |t| {
            t.span.offset - usize::from(matches!(t.token, Token::StringLit(_)))
        })
    }

    fn end(&mut self) -> usize {
        let next = self.start();
        self.ends
            .partition_point(|&end| end <= next)
            .checked_sub(1)
            .map_or(0, |i| self.ends[i])
    }

    fn take<T: Parse>(&mut self) -> Result<(T, Range<usize>)> {
        let start = self.start();
        let value = self.input.parse::<T>()?;
        Ok((value, start..self.end()))
    }

    fn nodes(
        &mut self,
        prefix: &str,
        out: &mut Vec<Node>,
        nested: bool,
        mut ordinal: usize,
    ) -> Result<usize> {
        while !self.input.is_empty() && !(nested && self.input.peek::<RBracket>()) {
            let path = format!("{prefix}{ordinal}");
            let index = out.len();
            let start = self.start();
            let line = self
                .input
                .peek_token()
                .context("entidade esperada")?
                .span
                .line;
            out.push(Node {
                path: path.clone(),
                name: None,
                tag: None,
                id: None,
                line,
                components: Vec::new(),
                bodies: Vec::new(),
                children: Vec::new(),
                range: start..start,
            });
            let mut children = 0;
            while !(self.input.is_empty()
                || self.input.peek::<DoubleMinus>()
                || self.input.peek::<RBracket>())
            {
                if self.input.peek::<Hash>() || self.input.peek::<Colon>() {
                    if let BsnEntry::Name(name) = self.input.parse::<BsnEntry>()? {
                        out[index].name = Some(name.0);
                    }
                    continue;
                }
                if self.input.peek::<At>() {
                    self.input.parse::<At>()?;
                    if self.input.peek::<StringLit>() {
                        self.input.parse::<StringLit>()?;
                        continue;
                    }
                }
                let component = self.input.parse::<TypePath>()?.to_type_path();
                if self.input.peek::<LBracket>() {
                    self.input.parse::<LBracket>()?;
                    let first = children;
                    children = self.nodes(&format!("{path}/"), out, true, children)?;
                    if matches!(
                        component.as_str(),
                        "bevy_ecs::hierarchy::Children" | "Children"
                    ) {
                        let close = self.start();
                        let end = self.end();
                        let ended_separator = self.text[..end].ends_with("--");
                        out[index]
                            .children
                            .push((close, children > first && !ended_separator));
                    }
                    self.input.parse::<RBracket>()?;
                    continue;
                }
                let (body, range) = self.take::<BsnFields>()?;
                let node = &mut out[index];
                if component == WORLD_NODE
                    && let BsnFields::NamedFields(fields) = &body
                {
                    node.tag = fields.iter().find_map(|f| match &f.value {
                        BsnValue::String(tag) if f.name == "tag" => Some(tag.clone()),
                        _ => None,
                    });
                }
                if component == SCENE_ID
                    && let BsnFields::UnnamedFields(fields) = &body
                    && let [BsnValue::Int(id)] = fields.as_slice()
                {
                    node.id = Some(id.to_string());
                }
                node.components.push(component);
                node.bodies.push(range);
            }
            let end = self.end();
            ensure!(end > start, "entidade vazia na linha {line}");
            out[index].range.end = end;
            ordinal += 1;
            if !self.input.peek::<DoubleMinus>() {
                break;
            }
            self.input.parse::<DoubleMinus>()?;
        }
        Ok(ordinal)
    }
}

pub fn index(text: &str) -> Result<Vec<Node>> {
    let mut cursor = Cursor::new(text)?;
    let mut nodes = Vec::new();
    cursor.nodes("", &mut nodes, false, 0)?;
    ensure!(
        cursor.input.is_empty(),
        "token inesperado no byte {}",
        cursor.start()
    );
    Ok(nodes)
}

pub fn select<'a>(nodes: &'a [Node], selector: &str) -> Result<&'a Node> {
    let mut found = nodes.iter().filter(|n| {
        if let Some(id) = selector.strip_prefix("id:") {
            n.id.as_deref() == Some(id)
        } else if let Some(name) = selector.strip_prefix("name:") {
            n.name.as_deref() == Some(name)
        } else {
            n.path == selector
        }
    });
    let first = found
        .next()
        .with_context(|| format!("nó inexistente: {selector}"))?;
    ensure!(
        found.next().is_none(),
        "seletor ambíguo: {selector}; escolha o caminho de inspect"
    );
    Ok(first)
}

impl Node {
    fn body(&self, component: &str) -> Result<Range<usize>> {
        let mut found = self
            .components
            .iter()
            .zip(&self.bodies)
            .filter(|(name, _)| {
                *name == component
                    || (!component.contains("::") && name.rsplit("::").next() == Some(component))
            });
        let (_, range) = found
            .next()
            .with_context(|| format!("componente {component} ausente em {}", self.path))?;
        ensure!(found.next().is_none(), "componente ambíguo: {component}");
        ensure!(range.start < range.end, "componente unitário: {component}");
        Ok(range.clone())
    }
}

// Read the native grammar one field at a time, retaining its source span.
fn named(text: &str, body: Range<usize>, key: &str) -> Result<Range<usize>> {
    let base = body.start;
    let mut c = Cursor::new(&text[body])?;
    c.input
        .parse::<LBrace>()
        .with_context(|| format!("esperava struct com o campo {key}"))?;
    let mut found = None;
    while !c.input.peek::<RBrace>() {
        let field = c.input.parse::<Ident>()?.0;
        c.input.parse::<Colon>()?;
        let (_, range) = c.take::<BsnValue>()?;
        if field == key {
            ensure!(found.is_none(), "campo duplicado: {key}");
            found = Some(base + range.start..base + range.end);
        }
        if !c.input.peek::<RBrace>() {
            c.input.parse::<Comma>()?;
        }
    }
    found.with_context(|| format!("campo ausente: {key}"))
}

fn struct_body(text: &str, value: Range<usize>) -> Result<Range<usize>> {
    let mut c = Cursor::new(&text[value.clone()])?;
    if c.input.peek::<At>() {
        c.input.parse::<At>()?;
    }
    c.input.parse::<TypePath>()?;
    Ok(value.start + c.start()..value.end)
}

pub fn field(text: &str, node: &Node, path: &str) -> Result<Range<usize>> {
    let (component, fields) = path
        .split_once('.')
        .context("use Componente.campo ou Componente.campo.subcampo")?;
    let mut body = node.body(component)?;
    let mut parts = fields.split('.').peekable();
    while let Some(key) = parts.next() {
        let range = named(text, body, key)?;
        if parts.peek().is_none() {
            return Ok(range);
        }
        body = struct_body(text, range)?;
    }
    bail!("campo vazio")
}

pub fn value(text: &str) -> Result<()> {
    let mut c = Cursor::new(text)?;
    let (_, span) = c.take::<BsnValue>()?;
    ensure!(
        c.input.is_empty() && text.trim() == &text[span],
        "esperava um único valor BSN"
    );
    Ok(())
}

pub fn attribute(
    text: &str,
    node: &Node,
    key: &str,
    value: &str,
) -> Result<(Range<usize>, String)> {
    let attrs = named(text, node.body(WORLD_NODE)?, "attrs")?;
    let mut c = Cursor::new(&text[attrs.clone()])?;
    c.input.parse::<LBracket>()?;
    let after_open = attrs.start + c.end();
    let mut found = None;
    while !c.input.peek::<RBracket>() {
        let (v, r) = c.take::<BsnValue>()?;
        if let BsnValue::Type(ty) = v
            && let BsnFields::NamedFields(fields) = ty.fields
            && fields
                .iter()
                .any(|f| f.name == "key" && f.value == BsnValue::String(key.into()))
        {
            ensure!(found.is_none(), "atributo duplicado: {key}");
            let obj = struct_body(text, attrs.start + r.start..attrs.start + r.end)?;
            found = Some(named(text, obj, "value")?);
        }
        if !c.input.peek::<RBracket>() {
            c.input.parse::<Comma>()?;
        }
    }
    let quote = |v: &str| jackdaw_bsn::emit_value_text(&jackdaw_bsn::BsnValue::String(v.into()));
    if let Some(range) = found {
        return Ok((range, quote(value)));
    }
    let indent = text[..attrs.start]
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|&c| c == ' ' || c == '\t')
        .collect::<String>();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let insert = format!(
        "{newline}{indent}    bevy_world_xml::bsn::WorldProp {{ key: {}, value: {} }},",
        quote(key),
        quote(value)
    );
    Ok((after_open..after_open, insert))
}

pub fn insert_child(text: &str, parent: &Node, fragment: &str) -> Result<(Range<usize>, String)> {
    let (offset, separator, new_list) = match parent.children.as_slice() {
        [] => (parent.range.end, false, true),
        [(p, sep)] => (*p, *sep, false),
        _ => bail!("várias listas Children no mesmo nó"),
    };
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent: String = text[..parent.range.start]
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    // Indenting every line would also change literal newlines in BSN strings.
    let mut block = String::new();
    if new_list {
        block.push_str(&format!("{nl}{indent}bevy_ecs::hierarchy::Children ["));
    }
    if separator {
        block.push_str("--");
    }
    block.push_str(&format!("{nl}{indent}    {}{nl}{indent}", fragment.trim()));
    if new_list {
        block.push(']');
    }
    Ok((offset..offset, block))
}
