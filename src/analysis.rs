use p4::ast::{AST, Control, Header, Parser, Struct, Extern, Action, Direction, Type};
use p4::lexer::Token;
use ropey::Rope;
use tower_lsp::lsp_types::*;

fn builtin_semantic_token_type(name: &str) -> Option<SemanticTokenType> {
    match name {
        "NoAction" => Some(SemanticTokenType::FUNCTION),
        "accept" | "reject" => Some(SemanticTokenType::KEYWORD),
        "packet_in"
        | "packet_out"
        | "void"
        | "tuple"
        | "match_kind"
        | "standard_metadata_t" => Some(SemanticTokenType::TYPE),
        "standard_metadata" => Some(SemanticTokenType::VARIABLE),
        "NoError"
        | "PacketTooShort"
        | "NoMatch"
        | "StackOutOfBounds"
        | "HeaderTooShort"
        | "ParserTimeout" => Some(SemanticTokenType::VARIABLE),
        _ => None,
    }
}

fn builtin_hover(name: &str) -> Option<&'static str> {
    match name {
        "NoAction" => Some("```p4\naction NoAction()\n```\nBuilt-in action."),
        "accept" => Some("```p4\naccept\n```\nBuilt-in parser state."),
        "reject" => Some("```p4\nreject\n```\nBuilt-in parser state."),
        "packet_in" => Some("```p4\npacket_in\n```\nBuilt-in type."),
        "packet_out" => Some("```p4\npacket_out\n```\nBuilt-in type."),
        "void" => Some("```p4\nvoid\n```\nBuilt-in type."),
        "tuple" => Some("```p4\ntuple\n```\nBuilt-in type."),
        "match_kind" => Some("```p4\nmatch_kind\n```\nBuilt-in type."),
        "standard_metadata_t" => Some("```p4\nstandard_metadata_t\n```\nCommon built-in type (architecture-defined)."),
        "standard_metadata" => Some("```p4\nstandard_metadata\n```\nCommon built-in variable (architecture-defined)."),
        "NoError" => Some("```p4\nNoError\n```\nCommon built-in error member (core library)."),
        "PacketTooShort" => Some("```p4\nPacketTooShort\n```\nCommon built-in error member (core library)."),
        "NoMatch" => Some("```p4\nNoMatch\n```\nCommon built-in error member (core library)."),
        "StackOutOfBounds" => Some("```p4\nStackOutOfBounds\n```\nCommon built-in error member (core library)."),
        "HeaderTooShort" => Some("```p4\nHeaderTooShort\n```\nCommon built-in error member (core library)."),
        "ParserTimeout" => Some("```p4\nParserTimeout\n```\nCommon built-in error member (core library)."),
        _ => None,
    }
}

pub fn get_builtin_hover(content: &Rope, position: Position) -> Option<String> {
    let line_idx = position.line as usize;
    let col_idx = position.character as usize;

    if line_idx >= content.len_lines() {
        return None;
    }

    let line = content.line(line_idx);
    let line_str: String = line.chars().collect();
    let (word, _) = extract_word_and_prefix(&line_str, col_idx);
    let word = word?;

    builtin_hover(&word).map(|s| s.to_string())
}

pub fn builtin_token_type_for_identifier(name: &str) -> Option<SemanticTokenType> {
    builtin_semantic_token_type(name)
}

pub fn get_hover_info(ast: &AST, content: &Rope, position: Position) -> Option<String> {
    let line_idx = position.line as usize;
    let col_idx = position.character as usize;

    if line_idx >= content.len_lines() {
        return None;
    }

    let line = content.line(line_idx);
    let line_str: String = line.chars().collect();

    let (word, prefix) = extract_word_and_prefix(&line_str, col_idx);
    let word = word?;

    if let Some(prefix) = prefix {
        if let Some(ty) = resolve_type_for_name(ast, &prefix, content, position) {
            return get_member_hover(ast, &ty, &word);
        }
    }

    if let Some(header) = ast.get_header(&word) {
        return Some(format_header_hover(header));
    }

    if let Some(s) = ast.get_struct(&word) {
        return Some(format_struct_hover(s));
    }

    if let Some(control) = ast.get_control(&word) {
        return Some(format_control_hover(control));
    }

    if let Some(parser) = ast.get_parser(&word) {
        return Some(format_parser_hover(parser));
    }

    if let Some(ext) = ast.get_extern(&word) {
        return Some(format_extern_hover(ext));
    }

    for control in &ast.controls {
        if let Some(action) = control.get_action(&word) {
            return Some(format_action_hover(action, &control.name));
        }
    }

    if let Some(hover) = format_scoped_hover(ast, &word, content, position) {
        return Some(hover);
    }

    if let Some(ty) = find_variable_type(ast, &word, content, position) {
        return Some(format!("```p4\n{} {}\n```", ty, word));
    }

    None
}

fn format_scoped_hover(
    ast: &AST,
    name: &str,
    content: &Rope,
    position: Position,
) -> Option<String> {
    if let Some((control, parser)) = find_enclosing_scope(ast, content, position) {
        if let Some(control) = control {
            if let Some(info) = control.names().get(name) {
                return Some(format_scoped_name_info(info, name));
            }
            for action in &control.actions {
                if let Some(info) = action.names().get(name) {
                    return Some(format_scoped_name_info(info, name));
                }
            }
        }
        if let Some(parser) = parser {
            if let Some(info) = parser.names().get(name) {
                return Some(format_scoped_name_info(info, name));
            }
        }
    }

    None
}

fn format_scoped_name_info(info: &p4::ast::NameInfo, name: &str) -> String {
    match info.decl {
        p4::ast::DeclarationInfo::Parameter(direction)
        | p4::ast::DeclarationInfo::ActionParameter(direction) => {
            let dir = format_direction(&direction);
            if dir.is_empty() {
                format!("```p4\n{} {}\n```", info.ty, name)
            } else {
                format!("```p4\n{} {} {}\n```", dir, info.ty, name)
            }
        }
        _ => format!("```p4\n{} {}\n```", info.ty, name),
    }
}

pub fn get_definition_location(
    ast: &AST,
    content: &Rope,
    position: Position,
) -> Option<Location> {
    let line_idx = position.line as usize;
    let col_idx = position.character as usize;

    if line_idx >= content.len_lines() {
        return None;
    }

    let line = content.line(line_idx);
    let line_str: String = line.chars().collect();

    let (word, prefix) = extract_word_and_prefix(&line_str, col_idx);
    let word = word?;

    if let Some(prefix) = prefix {
        if let Some(ty) = resolve_type_for_name(ast, &prefix, content, position) {
            if let Some(token) = resolve_member_token(ast, &ty, &word) {
                return token_to_location(&token);
            }
        }
    }

    if let Some(token) = resolve_global_token(ast, &word) {
        return token_to_location(&token);
    }

    if let Some(token) = resolve_scoped_token(ast, &word, content, position) {
        return token_to_location(&token);
    }

    None
}

fn resolve_member_token(ast: &AST, ty: &Type, member_name: &str) -> Option<Token> {
    match ty {
        Type::UserDefined(type_name) => {
            if let Some(s) = ast.get_struct(type_name) {
                for m in &s.members {
                    if m.name == member_name {
                        return Some(m.token.clone());
                    }
                }
            }
            if let Some(h) = ast.get_header(type_name) {
                for m in &h.members {
                    if m.name == member_name {
                        return Some(m.token.clone());
                    }
                }
                if member_name == "isValid" || member_name == "setValid" || member_name == "setInvalid" {
                    return Some(h.token.clone());
                }
            }
            if let Some(e) = ast.get_extern(type_name) {
                for m in &e.methods {
                    if m.name == member_name {
                        return Some(m.token.clone());
                    }
                }
            }
            None
        }
        Type::Table => None,
        _ => None,
    }
}

fn resolve_global_token(ast: &AST, name: &str) -> Option<Token> {
    if let Some(header) = ast.get_header(name) {
        return Some(header.token.clone());
    }
    if let Some(s) = ast.get_struct(name) {
        return Some(s.token.clone());
    }
    if let Some(control) = ast.get_control(name) {
        return Some(control.token.clone());
    }
    if let Some(parser) = ast.get_parser(name) {
        return Some(parser.token.clone());
    }
    if let Some(ext) = ast.get_extern(name) {
        return Some(ext.token.clone());
    }
    for td in &ast.typedefs {
        if td.name == name {
            return Some(td.token.clone());
        }
    }
    for c in &ast.constants {
        if c.name == name {
            return Some(c.token.clone());
        }
    }
    for pkg in &ast.packages {
        if pkg.name == name {
            return Some(pkg.token.clone());
        }
    }
    for control in &ast.controls {
        if let Some(action) = control.get_action(name) {
            return Some(action.token.clone());
        }
        if let Some(table) = control.get_table(name) {
            return Some(table.token.clone());
        }
    }
    for parser in &ast.parsers {
        for state in &parser.states {
            if state.name == name {
                return Some(state.token.clone());
            }
        }
    }
    None
}

fn resolve_scoped_token(
    ast: &AST,
    name: &str,
    content: &Rope,
    position: Position,
) -> Option<Token> {
    if let Some((control, parser)) = find_enclosing_scope(ast, content, position) {
        if let Some(control) = control {
            if let Some(info) = control.names().get(name) {
                return Some(info.token.clone());
            }
            for action in &control.actions {
                if let Some(info) = action.names().get(name) {
                    return Some(info.token.clone());
                }
            }
        }
        if let Some(parser) = parser {
            if let Some(info) = parser.names().get(name) {
                return Some(info.token.clone());
            }
        }
    }
    None
}

fn token_to_location(token: &Token) -> Option<Location> {
    let path = token.file.as_str();
    let uri = Url::from_file_path(path).ok()?;
    let line = token.line as u32;
    let col = token.col as u32;
    Some(Location {
        uri,
        range: Range {
            start: Position {
                line,
                character: col,
            },
            end: Position {
                line,
                character: col + 1,
            },
        },
    })
}

fn get_member_hover(ast: &AST, ty: &Type, member_name: &str) -> Option<String> {
    match ty {
        Type::UserDefined(type_name) => {
            if let Some(s) = ast.get_struct(type_name) {
                for m in &s.members {
                    if m.name == member_name {
                        return Some(format!("```p4\n{} {}.{}\n```\nMember of struct `{}`", m.ty, type_name, m.name, type_name));
                    }
                }
            }
            if let Some(h) = ast.get_header(type_name) {
                for m in &h.members {
                    if m.name == member_name {
                        return Some(format!("```p4\n{} {}.{}\n```\nMember of header `{}`", m.ty, type_name, m.name, type_name));
                    }
                }
                if member_name == "isValid" || member_name == "setValid" || member_name == "setInvalid" {
                    return Some(format!("```p4\n{}()\n```\nHeader method", member_name));
                }
            }
            if let Some(e) = ast.get_extern(type_name) {
                for m in &e.methods {
                    if m.name == member_name {
                        let params: Vec<String> = m.parameters.iter()
                            .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
                            .collect();
                        return Some(format!("```p4\n{} {}({})\n```\nMethod of extern `{}`", m.return_type, m.name, params.join(", "), type_name));
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub fn get_completions(ast: &AST) -> Vec<CompletionItem> {
    let mut completions = Vec::new();

    for header in &ast.headers {
        completions.push(CompletionItem {
            label: header.name.clone(),
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some("header".to_string()),
            ..Default::default()
        });
    }

    for s in &ast.structs {
        completions.push(CompletionItem {
            label: s.name.clone(),
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some("struct".to_string()),
            ..Default::default()
        });
    }

    for control in &ast.controls {
        completions.push(CompletionItem {
            label: control.name.clone(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("control".to_string()),
            ..Default::default()
        });

        for action in &control.actions {
            completions.push(CompletionItem {
                label: action.name.clone(),
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("action in {}", control.name)),
                ..Default::default()
            });
        }

        for table in &control.tables {
            completions.push(CompletionItem {
                label: table.name.clone(),
                kind: Some(CompletionItemKind::VARIABLE),
                detail: Some(format!("table in {}", control.name)),
                ..Default::default()
            });
        }
    }

    for parser in &ast.parsers {
        completions.push(CompletionItem {
            label: parser.name.clone(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("parser".to_string()),
            ..Default::default()
        });
    }

    for ext in &ast.externs {
        completions.push(CompletionItem {
            label: ext.name.clone(),
            kind: Some(CompletionItemKind::INTERFACE),
            detail: Some("extern".to_string()),
            ..Default::default()
        });
    }

    add_keyword_completions(&mut completions);

    completions
}

pub fn get_completions_with_context(
    ast: &AST,
    content: &Rope,
    position: Position,
) -> Vec<CompletionItem> {
    let line_idx = position.line as usize;
    let col_idx = position.character as usize;

    if line_idx >= content.len_lines() {
        return get_completions(ast);
    }

    let line = content.line(line_idx);
    let line_str: String = line.chars().collect();

    if let Some(prefix) = get_dot_prefix(&line_str, col_idx) {
        if let Some(ty) = resolve_type_for_name(ast, &prefix, content, position) {
            return get_member_completions(ast, &ty);
        }
    }

    let mut completions = get_completions(ast);

    if let Some((control, parser)) = find_enclosing_scope(ast, content, position) {
        if let Some(control) = control {
            for param in &control.parameters {
                completions.push(CompletionItem {
                    label: param.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("{} (parameter)", param.ty)),
                    ..Default::default()
                });
            }
            for var in &control.variables {
                completions.push(CompletionItem {
                    label: var.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("{}", var.ty)),
                    ..Default::default()
                });
            }
            for action in &control.actions {
                completions.push(CompletionItem {
                    label: action.name.clone(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some("action".to_string()),
                    ..Default::default()
                });
            }
            for table in &control.tables {
                completions.push(CompletionItem {
                    label: table.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some("table".to_string()),
                    ..Default::default()
                });
            }
        }
        if let Some(parser) = parser {
            for param in &parser.parameters {
                completions.push(CompletionItem {
                    label: param.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("{} (parameter)", param.ty)),
                    ..Default::default()
                });
            }
        }
    }

    completions
}

fn get_member_completions(ast: &AST, ty: &Type) -> Vec<CompletionItem> {
    let mut completions = Vec::new();

    match ty {
        Type::UserDefined(type_name) => {
            if let Some(s) = ast.get_struct(type_name) {
                for member in &s.members {
                    completions.push(CompletionItem {
                        label: member.name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!("{}", member.ty)),
                        documentation: Some(Documentation::String(format!("Member of struct {}", type_name))),
                        ..Default::default()
                    });
                }
            }
            if let Some(h) = ast.get_header(type_name) {
                for member in &h.members {
                    completions.push(CompletionItem {
                        label: member.name.clone(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!("{}", member.ty)),
                        documentation: Some(Documentation::String(format!("Member of header {}", type_name))),
                        ..Default::default()
                    });
                }
                completions.push(CompletionItem {
                    label: "isValid".to_string(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some("bool".to_string()),
                    insert_text: Some("isValid()".to_string()),
                    documentation: Some(Documentation::String("Check if header is valid".to_string())),
                    ..Default::default()
                });
                completions.push(CompletionItem {
                    label: "setValid".to_string(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some("void".to_string()),
                    insert_text: Some("setValid()".to_string()),
                    documentation: Some(Documentation::String("Mark header as valid".to_string())),
                    ..Default::default()
                });
                completions.push(CompletionItem {
                    label: "setInvalid".to_string(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some("void".to_string()),
                    insert_text: Some("setInvalid()".to_string()),
                    documentation: Some(Documentation::String("Mark header as invalid".to_string())),
                    ..Default::default()
                });
            }
            if let Some(e) = ast.get_extern(type_name) {
                for method in &e.methods {
                    let params: Vec<String> = method.parameters.iter()
                        .map(|p| format!("{} {}", p.ty, p.name))
                        .collect();
                    completions.push(CompletionItem {
                        label: method.name.clone(),
                        kind: Some(CompletionItemKind::METHOD),
                        detail: Some(format!("{} ({})", method.return_type, params.join(", "))),
                        ..Default::default()
                    });
                }
            }
        }
        Type::Table => {
            completions.push(CompletionItem {
                label: "apply".to_string(),
                kind: Some(CompletionItemKind::METHOD),
                detail: Some("Apply the table".to_string()),
                insert_text: Some("apply()".to_string()),
                ..Default::default()
            });
        }
        _ => {}
    }

    completions
}

fn get_dot_prefix(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if col == 0 || col > chars.len() {
        return None;
    }

    let search_col = col.saturating_sub(1);
    if search_col >= chars.len() || chars[search_col] != '.' {
        return None;
    }

    let end = search_col;
    let mut start = end;
    while start > 0 && is_identifier_char(chars[start - 1]) {
        start -= 1;
    }

    if start == end {
        return None;
    }

    Some(chars[start..end].iter().collect())
}

fn resolve_type_for_name(ast: &AST, name: &str, content: &Rope, position: Position) -> Option<Type> {
    let parts: Vec<&str> = name.split('.').collect();
    let base_name = parts[0];

    let mut current_type = find_variable_type(ast, base_name, content, position)?;

    for part in parts.iter().skip(1) {
        current_type = resolve_member_type(ast, &current_type, part)?;
    }

    Some(current_type)
}

fn resolve_member_type(ast: &AST, ty: &Type, member: &str) -> Option<Type> {
    match ty {
        Type::UserDefined(type_name) => {
            if let Some(s) = ast.get_struct(type_name) {
                for m in &s.members {
                    if m.name == member {
                        return Some(m.ty.clone());
                    }
                }
            }
            if let Some(h) = ast.get_header(type_name) {
                for m in &h.members {
                    if m.name == member {
                        return Some(m.ty.clone());
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn find_variable_type(ast: &AST, name: &str, content: &Rope, position: Position) -> Option<Type> {
    if let Some((control, parser)) = find_enclosing_scope(ast, content, position) {
        if let Some(control) = control {
            for param in &control.parameters {
                if param.name == name {
                    return Some(param.ty.clone());
                }
            }
            for var in &control.variables {
                if var.name == name {
                    return Some(var.ty.clone());
                }
            }
            for table in &control.tables {
                if table.name == name {
                    return Some(Type::Table);
                }
            }
        }
        if let Some(parser) = parser {
            for param in &parser.parameters {
                if param.name == name {
                    return Some(param.ty.clone());
                }
            }
        }
    }

    if let Some(s) = ast.get_struct(name) {
        return Some(Type::UserDefined(s.name.clone()));
    }
    if let Some(h) = ast.get_header(name) {
        return Some(Type::UserDefined(h.name.clone()));
    }
    if let Some(e) = ast.get_extern(name) {
        return Some(Type::UserDefined(e.name.clone()));
    }

    None
}

fn find_enclosing_scope<'a>(
    ast: &'a AST,
    content: &Rope,
    position: Position,
) -> Option<(Option<&'a Control>, Option<&'a Parser>)> {
    let target_line = position.line as usize;

    for control in &ast.controls {
        if is_position_in_control(control, content, target_line) {
            return Some((Some(control), None));
        }
    }

    for parser in &ast.parsers {
        if is_position_in_parser(parser, content, target_line) {
            return Some((None, Some(parser)));
        }
    }

    None
}

fn is_position_in_control(control: &Control, content: &Rope, target_line: usize) -> bool {
    let content_str: String = content.chars().collect();
    let control_pattern = format!("control {}", control.name);

    if let Some(start_pos) = content_str.find(&control_pattern) {
        let start_line = content_str[..start_pos].matches('\n').count();

        let remaining = &content_str[start_pos..];
        let mut brace_count = 0;
        let mut end_line = start_line;
        let mut found_open = false;

        for c in remaining.chars() {
            if c == '{' {
                brace_count += 1;
                found_open = true;
            } else if c == '}' {
                brace_count -= 1;
                if found_open && brace_count == 0 {
                    break;
                }
            } else if c == '\n' {
                end_line += 1;
            }
        }

        return target_line >= start_line && target_line <= end_line;
    }

    false
}

fn is_position_in_parser(parser: &Parser, content: &Rope, target_line: usize) -> bool {
    let content_str: String = content.chars().collect();
    let parser_pattern = format!("parser {}", parser.name);

    if let Some(start_pos) = content_str.find(&parser_pattern) {
        let start_line = content_str[..start_pos].matches('\n').count();

        let remaining = &content_str[start_pos..];
        let mut brace_count = 0;
        let mut end_line = start_line;
        let mut found_open = false;

        for c in remaining.chars() {
            if c == '{' {
                brace_count += 1;
                found_open = true;
            } else if c == '}' {
                brace_count -= 1;
                if found_open && brace_count == 0 {
                    break;
                }
            } else if c == '\n' {
                end_line += 1;
            }
        }

        return target_line >= start_line && target_line <= end_line;
    }

    false
}

pub fn extract_word_and_prefix(line: &str, col: usize) -> (Option<String>, Option<String>) {
    let chars: Vec<char> = line.chars().collect();

    if chars.is_empty() {
        return (None, None);
    }

    // VSCode frequently sends positions that sit *between* characters (e.g. at the end of an
    // identifier). Make a best-effort attempt to snap the column onto a nearby identifier.
    let mut idx = col;
    if idx >= chars.len() {
        idx = chars.len() - 1;
    }
    if !is_identifier_char(chars[idx]) {
        if idx > 0 && is_identifier_char(chars[idx - 1]) {
            idx -= 1;
        } else if idx + 1 < chars.len() && is_identifier_char(chars[idx + 1]) {
            idx += 1;
        } else {
            return (None, None);
        }
    }

    let mut start = idx;
    while start > 0 && is_identifier_char(chars[start - 1]) {
        start -= 1;
    }

    let mut end = idx + 1;
    while end < chars.len() && is_identifier_char(chars[end]) {
        end += 1;
    }

    let word: String = chars[start..end].iter().collect();

    let mut prefix = None;
    if start > 0 && chars[start - 1] == '.' {
        let prefix_end = start - 1;
        let mut prefix_start = prefix_end;
        while prefix_start > 0 && (is_identifier_char(chars[prefix_start - 1]) || chars[prefix_start - 1] == '.') {
            prefix_start -= 1;
        }
        if prefix_start < prefix_end {
            prefix = Some(chars[prefix_start..prefix_end].iter().collect());
        }
    }

    (Some(word), prefix)
}

fn is_identifier_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn format_direction(d: &Direction) -> &'static str {
    match d {
        Direction::In => "in",
        Direction::Out => "out",
        Direction::InOut => "inout",
        Direction::Unspecified => "",
    }
}

fn format_header_hover(header: &Header) -> String {
    let mut result = format!("```p4\nheader {} {{\n", header.name);
    for member in &header.members {
        result.push_str(&format!("    {} {};\n", member.ty, member.name));
    }
    result.push_str("}\n```");
    result
}

fn format_struct_hover(s: &Struct) -> String {
    let mut result = format!("```p4\nstruct {} {{\n", s.name);
    for member in &s.members {
        result.push_str(&format!("    {} {};\n", member.ty, member.name));
    }
    result.push_str("}\n```");
    result
}

fn format_control_hover(control: &Control) -> String {
    let params: Vec<String> = control.parameters.iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!("```p4\ncontrol {}({})\n```", control.name, params.join(", "))
}

fn format_parser_hover(parser: &Parser) -> String {
    let params: Vec<String> = parser.parameters.iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!("```p4\nparser {}({})\n```", parser.name, params.join(", "))
}

fn format_extern_hover(ext: &Extern) -> String {
    let mut result = format!("```p4\nextern {} {{\n", ext.name);
    for method in &ext.methods {
        let params: Vec<String> = method.parameters.iter()
            .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
            .collect();
        result.push_str(&format!("    {} {}({});\n", method.return_type, method.name, params.join(", ")));
    }
    result.push_str("}\n```");
    result
}

fn format_action_hover(action: &Action, control_name: &str) -> String {
    let params: Vec<String> = action.parameters.iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!("```p4\n// in control {}\naction {}({})\n```", control_name, action.name, params.join(", "))
}

fn add_keyword_completions(completions: &mut Vec<CompletionItem>) {
    let keywords = [
        ("action", "Define an action"),
        ("apply", "Apply block"),
        ("control", "Define a control block"),
        ("default_action", "Set default action for table"),
        ("else", "Else branch"),
        ("exact", "Exact match type"),
        ("extern", "Extern declaration"),
        ("header", "Define a header type"),
        ("if", "Conditional statement"),
        ("in", "Input direction"),
        ("inout", "Input/output direction"),
        ("key", "Table key definition"),
        ("lpm", "Longest prefix match"),
        ("out", "Output direction"),
        ("package", "Package declaration"),
        ("parser", "Define a parser"),
        ("range", "Range match type"),
        ("return", "Return statement"),
        ("select", "Select statement in parser"),
        ("size", "Table size"),
        ("state", "Parser state"),
        ("struct", "Define a struct type"),
        ("table", "Define a table"),
        ("ternary", "Ternary match type"),
        ("transition", "Parser state transition"),
        ("typedef", "Type definition"),
    ];

    for (kw, doc) in keywords {
        completions.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(doc.to_string()),
            ..Default::default()
        });
    }

    let types = ["bit", "bool", "error", "int", "varbit"];
    for ty in types {
        completions.push(CompletionItem {
            label: ty.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some("Built-in type".to_string()),
            ..Default::default()
        });
    }

    let builtins = [
        ("NoAction", CompletionItemKind::FUNCTION, "Built-in action"),
        ("accept", CompletionItemKind::KEYWORD, "Built-in parser state"),
        ("reject", CompletionItemKind::KEYWORD, "Built-in parser state"),
        ("packet_in", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("packet_out", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("void", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("tuple", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("match_kind", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("string", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        ("NoError", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("PacketTooShort", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("NoMatch", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("StackOutOfBounds", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("HeaderTooShort", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("ParserTimeout", CompletionItemKind::CONSTANT, "Common built-in error member"),
        ("standard_metadata_t", CompletionItemKind::TYPE_PARAMETER, "Common architecture type"),
        ("standard_metadata", CompletionItemKind::VARIABLE, "Common architecture variable"),
    ];

    for (label, kind, detail) in builtins {
        completions.push(CompletionItem {
            label: label.to_string(),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }
}

pub fn get_keyword_completions() -> Vec<CompletionItem> {
    let mut completions = Vec::new();
    add_keyword_completions(&mut completions);
    completions
}
