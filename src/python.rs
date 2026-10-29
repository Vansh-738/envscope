use crate::{Access, Report, Unresolved, valid_name};
use std::collections::BTreeMap;
use tree_sitter::{Node, Parser};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Binding {
    Os,
    Environ,
    Getenv,
}

type Bindings = BTreeMap<String, Option<Binding>>;

fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn bind(map: &mut Bindings, name: &str, value: Option<Binding>) {
    map.entry(name.to_string())
        .and_modify(|old| {
            if *old != value {
                *old = None;
            }
        })
        .or_insert(value);
}

fn target_names(node: Node<'_>, source: &str, map: &mut Bindings) {
    match node.kind() {
        "identifier" => {
            map.insert(text(node, source).into(), None);
        }
        "attribute" | "subscript" => {}
        "default_parameter" | "typed_default_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                target_names(name, source, map);
            }
        }
        "typed_parameter" => {
            if let Some(first) = children(node).first() {
                target_names(*first, source, map);
            }
        }
        _ => {
            for child in children(node) {
                target_names(child, source, map);
            }
        }
    }
}

fn import_bindings(node: Node<'_>, source: &str, map: &mut Bindings) {
    let from_os = node.kind() == "import_from_statement"
        && node
            .child_by_field_name("module_name")
            .is_some_and(|m| text(m, source) == "os");
    let module = node.child_by_field_name("module_name");
    for item in children(node) {
        if Some(item) == module {
            continue;
        }
        let (name_node, alias) = if item.kind() == "aliased_import" {
            let Some(name) = item.child_by_field_name("name") else {
                continue;
            };
            (name, item.child_by_field_name("alias"))
        } else if matches!(item.kind(), "dotted_name" | "identifier") {
            (item, None)
        } else {
            continue;
        };
        let imported = text(name_node, source);
        let local = alias
            .map(|a| text(a, source))
            .unwrap_or_else(|| imported.split('.').next().unwrap_or(imported));
        let binding = if from_os {
            match imported {
                "environ" => Some(Binding::Environ),
                "getenv" => Some(Binding::Getenv),
                _ => None,
            }
        } else if node.kind() == "import_statement"
            && (imported == "os" || imported.starts_with("os."))
            && (alias.is_none() || imported == "os")
        {
            Some(Binding::Os)
        } else {
            None
        };
        bind(map, local, binding);
    }
}

// Collect only bindings owned by this scope. Conflicting bindings are deliberately
// left unresolved instead of guessing which value reaches a particular lookup.
fn collect(node: Node<'_>, source: &str, map: &mut Bindings) {
    match node.kind() {
        "import_statement" | "import_from_statement" => {
            import_bindings(node, source, map);
            return;
        }
        "function_definition" | "class_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                map.insert(text(name, source).into(), None);
            }
            return;
        }
        "lambda"
        | "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => return,
        "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
            if let Some(left) = node.child_by_field_name("left") {
                target_names(left, source, map);
            }
        }
        "named_expression" => {
            if let Some(name) = node.child_by_field_name("name") {
                target_names(name, source, map);
            }
        }
        "as_pattern" => {
            if let Some(alias) = node.child_by_field_name("alias") {
                target_names(alias, source, map);
            }
        }
        "global_statement" | "nonlocal_statement" | "delete_statement" => {
            for child in children(node) {
                target_names(child, source, map);
            }
        }
        _ => {}
    }
    for child in children(node) {
        collect(child, source, map);
    }
}

fn scoped(
    body: Node<'_>,
    parameters: Option<Node<'_>>,
    source: &str,
    parent: &Bindings,
) -> Bindings {
    let mut local = Bindings::new();
    collect(body, source, &mut local);
    if let Some(parameters) = parameters {
        target_names(parameters, source, &mut local);
    }
    let mut merged = parent.clone();
    merged.extend(local);
    merged
}

fn resolves(node: Node<'_>, source: &str, map: &Bindings, wanted: Binding) -> bool {
    if node.kind() == "identifier" {
        return map.get(text(node, source)) == Some(&Some(wanted));
    }
    if node.kind() == "attribute" {
        let Some(object) = node.child_by_field_name("object") else {
            return false;
        };
        let Some(attribute) = node.child_by_field_name("attribute") else {
            return false;
        };
        let attr = text(attribute, source);
        return resolves(object, source, map, Binding::Os)
            && matches!(
                (wanted, attr),
                (Binding::Environ, "environ") | (Binding::Getenv, "getenv")
            );
    }
    false
}

fn literal(node: Node<'_>, source: &str) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, source);
    let offset = raw.find(['\'', '"'])?;
    let prefix = &raw[..offset];
    if !prefix.chars().all(|c| matches!(c, 'r' | 'R' | 'u' | 'U')) {
        return None;
    }
    let quoted = &raw[offset..];
    let quote = quoted.chars().next()?;
    let width = if quoted.starts_with(&quote.to_string().repeat(3)) {
        3
    } else {
        1
    };
    let value = quoted.get(width..quoted.len().checked_sub(width)?)?;
    valid_name(value).then(|| value.to_string())
}

fn write_only(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    (parent.kind() == "assignment" && parent.child_by_field_name("left") == Some(node))
        || parent.kind() == "delete_statement"
}

struct Analyzer<'a> {
    source: &'a str,
    file: &'a str,
    report: &'a mut Report,
}

impl Analyzer<'_> {
    fn record(&mut self, site: Node<'_>, key: Option<Node<'_>>, kind: &str) {
        if let Some(name) = key.and_then(|key| literal(key, self.source)) {
            self.report.accesses.push(Access {
                name,
                file: self.file.into(),
                line: site.start_position().row + 1,
                column: site.start_position().column + 1,
                kind: kind.into(),
            });
        } else {
            self.report.unresolved.push(Unresolved {
                file: self.file.into(),
                line: site.start_position().row + 1,
                reason: "lookup needs a literal variable name; expression was not evaluated".into(),
            });
        }
    }

    fn visit(&mut self, node: Node<'_>, bindings: &Bindings, method_parent: Option<&Bindings>) {
        match node.kind() {
            "function_definition" | "lambda" => {
                let parameters = node.child_by_field_name("parameters");
                let body = node.child_by_field_name("body");
                if let Some(body) = body {
                    // Defaults, annotations and decorators use the enclosing scope.
                    for child in children(node) {
                        if child != body {
                            self.visit(child, bindings, method_parent);
                        }
                    }
                    let local = scoped(
                        body,
                        parameters,
                        self.source,
                        method_parent.unwrap_or(bindings),
                    );
                    self.visit(body, &local, None);
                }
                return;
            }
            "class_definition" => {
                if let Some(body) = node.child_by_field_name("body") {
                    for child in children(node) {
                        if child != body {
                            self.visit(child, bindings, method_parent);
                        }
                    }
                    let local = scoped(body, None, self.source, bindings);
                    self.visit(body, &local, Some(method_parent.unwrap_or(bindings)));
                }
                return;
            }
            "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression" => {
                let mut local = bindings.clone();
                for clause in children(node) {
                    if clause.kind() == "for_in_clause"
                        && let Some(left) = clause.child_by_field_name("left")
                    {
                        target_names(left, self.source, &mut local);
                    }
                }
                for child in children(node) {
                    self.visit(child, &local, method_parent);
                }
                return;
            }
            "subscript" if !write_only(node) => {
                if node
                    .child_by_field_name("value")
                    .is_some_and(|v| resolves(v, self.source, bindings, Binding::Environ))
                {
                    let mut cursor = node.walk();
                    let keys: Vec<_> = node
                        .children_by_field_name("subscript", &mut cursor)
                        .collect();
                    self.record(node, (keys.len() == 1).then(|| keys[0]), "subscript");
                }
            }
            "call" => {
                if let Some(function) = node.child_by_field_name("function") {
                    let getenv = resolves(function, self.source, bindings, Binding::Getenv);
                    let environ_get = function.kind() == "attribute"
                        && function
                            .child_by_field_name("attribute")
                            .is_some_and(|n| text(n, self.source) == "get")
                        && function
                            .child_by_field_name("object")
                            .is_some_and(|n| resolves(n, self.source, bindings, Binding::Environ));
                    if getenv || environ_get {
                        let args = node
                            .child_by_field_name("arguments")
                            .map(children)
                            .unwrap_or_default();
                        let key = args
                            .iter()
                            .copied()
                            .find(|a| a.kind() != "comment")
                            .and_then(|arg| {
                                if arg.kind() == "keyword_argument" {
                                    args.iter()
                                        .find(|a| {
                                            a.child_by_field_name("name")
                                                .is_some_and(|n| text(n, self.source) == "key")
                                        })
                                        .and_then(|a| a.child_by_field_name("value"))
                                } else {
                                    Some(arg)
                                }
                            });
                        self.record(node, key, if getenv { "getenv" } else { "get" });
                    }
                }
            }
            _ => {}
        }
        for child in children(node) {
            self.visit(child, bindings, method_parent);
        }
    }
}

pub fn analyze(source: &str, file: &str, report: &mut Report) {
    let mut parser = Parser::new();
    if let Err(error) = parser.set_language(&tree_sitter_python::LANGUAGE.into()) {
        report
            .errors
            .push(format!("cannot load Python grammar: {error}"));
        return;
    }
    let Some(tree) = parser.parse(source, None) else {
        report.errors.push(format!("cannot parse {file}"));
        return;
    };
    if tree.root_node().has_error() {
        report.errors.push(format!(
            "{file}: Python syntax error; file was not analyzed"
        ));
        return;
    }
    let bindings = scoped(tree.root_node(), None, source, &Bindings::new());
    Analyzer {
        source,
        file,
        report,
    }
    .visit(tree.root_node(), &bindings, None);
}
