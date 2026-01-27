use p4::ast::Typedef;
use p4::ast::{Action, Control, Direction, Extern, Header, Parser, Struct, Type, AST};
use p4::lexer::Token;
use ropey::Rope;
use tower_lsp::lsp_types::*;

fn builtin_semantic_token_type(name: &str) -> Option<SemanticTokenType> {
    match name {
        // Core library: actions
        "NoAction" => Some(SemanticTokenType::FUNCTION),

        // Core library: parser states and control flow keywords
        "accept" | "reject" | "exit" | "default" | "switch" | "verify" => {
            Some(SemanticTokenType::KEYWORD)
        }

        // Core library: extern types
        "packet_in" | "packet_out" => Some(SemanticTokenType::TYPE),

        // Core library: built-in types
        "void" | "tuple" | "match_kind" => Some(SemanticTokenType::TYPE),

        // Core library: error members (from core.p4)
        "NoError"
        | "PacketTooShort"
        | "NoMatch"
        | "StackOutOfBounds"
        | "HeaderTooShort"
        | "ParserTimeout"
        | "ParserInvalidArgument" => Some(SemanticTokenType::ENUM_MEMBER),

        // Core library: match_kind values (from core.p4)
        "exact" | "ternary" | "lpm" => Some(SemanticTokenType::ENUM_MEMBER),

        // Core library: extern methods (packet_in)
        "extract" | "lookahead" | "advance" | "length" => Some(SemanticTokenType::METHOD),

        // Core library: extern methods (packet_out)
        "emit" => Some(SemanticTokenType::METHOD),

        // Core library: header methods
        "isValid" | "setValid" | "setInvalid" => Some(SemanticTokenType::METHOD),

        // Core library: functions
        "static_assert" => Some(SemanticTokenType::FUNCTION),

        // v1model architecture: match_kind extensions
        "range" | "optional" | "selector" => Some(SemanticTokenType::ENUM_MEMBER),

        // v1model architecture: types
        "standard_metadata_t" => Some(SemanticTokenType::STRUCT),
        "CounterType" | "MeterType" | "CloneType" | "HashAlgorithm" => {
            Some(SemanticTokenType::TYPE)
        }

        // v1model architecture: variables
        "standard_metadata" => Some(SemanticTokenType::VARIABLE),

        // v1model architecture: enum values
        "packets" | "bytes" | "packets_and_bytes" => Some(SemanticTokenType::ENUM_MEMBER),

        // v1model architecture: externs
        "counter" | "direct_counter" | "meter" | "direct_meter" | "register" | "action_profile"
        | "action_selector" | "Checksum16" | "random" | "digest" | "resubmit" | "recirculate"
        | "clone" | "clone3" | "truncate" | "hash" | "mark_to_drop" | "log_msg" => {
            Some(SemanticTokenType::FUNCTION)
        }

        _ => None,
    }
}

fn builtin_hover(name: &str) -> Option<&'static str> {
    match name {
        // Core library: actions
        "NoAction" => Some("```p4\n@noWarn(\"unused\")\naction NoAction() {}\n```\nBuilt-in action that does nothing (core.p4)."),

        // Core library: parser states
        "accept" => Some("```p4\naccept\n```\nBuilt-in parser state indicating successful parsing."),
        "reject" => Some("```p4\nreject\n```\nBuilt-in parser state indicating parsing failure."),

        // Core library: extern types
        "packet_in" => Some("```p4\nextern packet_in {\n    void extract<T>(out T hdr);\n    void extract<T>(out T variableSizeHeader, in bit<32> variableFieldSizeInBits);\n    T lookahead<T>();\n    void advance(in bit<32> sizeInBits);\n    bit<32> length();\n}\n```\nBuilt-in extern representing incoming network packets (core.p4)."),
        "packet_out" => Some("```p4\nextern packet_out {\n    void emit<T>(in T hdr);\n}\n```\nBuilt-in extern for constructing output packets (core.p4)."),

        // Core library: types
        "void" => Some("```p4\nvoid\n```\nBuilt-in type representing no value."),
        "tuple" => Some("```p4\ntuple<T1, T2, ...>\n```\nBuilt-in tuple type."),
        "match_kind" => Some("```p4\nmatch_kind\n```\nBuilt-in type for table key match types (exact, ternary, lpm)."),

        // Core library: error members
        "NoError" => Some("```p4\nerror.NoError\n```\nNo error (core.p4)."),
        "PacketTooShort" => Some("```p4\nerror.PacketTooShort\n```\nNot enough bits in packet for 'extract' (core.p4)."),
        "NoMatch" => Some("```p4\nerror.NoMatch\n```\n'select' expression has no matches (core.p4)."),
        "StackOutOfBounds" => Some("```p4\nerror.StackOutOfBounds\n```\nReference to invalid element of a header stack (core.p4)."),
        "HeaderTooShort" => Some("```p4\nerror.HeaderTooShort\n```\nExtracting too many bits into a varbit field (core.p4)."),
        "ParserTimeout" => Some("```p4\nerror.ParserTimeout\n```\nParser execution time limit exceeded (core.p4)."),
        "ParserInvalidArgument" => Some("```p4\nerror.ParserInvalidArgument\n```\nParser operation was called with a value not supported by the implementation (core.p4)."),

        // Core library: match_kind values
        "exact" => Some("```p4\nmatch_kind exact\n```\nMatch bits exactly (core.p4)."),
        "ternary" => Some("```p4\nmatch_kind ternary\n```\nTernary match, using a mask (core.p4)."),
        "lpm" => Some("```p4\nmatch_kind lpm\n```\nLongest-prefix match (core.p4)."),

        // Core library: packet_in methods
        "extract" => Some("```p4\nvoid extract<T>(out T hdr)\nvoid extract<T>(out T variableSizeHeader, in bit<32> variableFieldSizeInBits)\n```\nRead a header from the packet and advance the cursor (core.p4)."),
        "lookahead" => Some("```p4\nT lookahead<T>()\n```\nRead bits from the packet without advancing the cursor (core.p4)."),
        "advance" => Some("```p4\nvoid advance(in bit<32> sizeInBits)\n```\nAdvance the packet cursor by the specified number of bits (core.p4)."),
        "length" => Some("```p4\nbit<32> length()\n```\nReturn packet length in bytes (core.p4)."),

        // Core library: packet_out methods
        "emit" => Some("```p4\nvoid emit<T>(in T hdr)\n```\nWrite header into the output packet, advancing cursor (core.p4)."),

        // Core library: header methods
        "isValid" => Some("```p4\nbool isValid()\n```\nReturn true if the header is valid."),
        "setValid" => Some("```p4\nvoid setValid()\n```\nMark the header as valid."),
        "setInvalid" => Some("```p4\nvoid setInvalid()\n```\nMark the header as invalid."),

        // Core library: functions
        "verify" => Some("```p4\nextern void verify(in bool check, in error toSignal)\n```\nCheck a predicate in the parser; if false, set parser error and transition to reject (core.p4)."),
        "static_assert" => Some("```p4\nextern bool static_assert(bool check, string message)\nextern bool static_assert(bool check)\n```\nEvaluate a boolean expression at compile time; stop compilation if false (core.p4)."),

        // Control flow
        "exit" => Some("```p4\nexit\n```\nStatement that causes immediate exit from the enclosing control block."),
        "this" => Some("```p4\nthis\n```\nReference to the enclosing object."),
        "default" => Some("```p4\ndefault\n```\nDefault case in select/switch statements."),

        // v1model architecture: match_kind extensions
        "range" => Some("```p4\nmatch_kind range\n```\nRange match (v1model)."),
        "optional" => Some("```p4\nmatch_kind optional\n```\nEither an exact match or a wildcard (v1model)."),
        "selector" => Some("```p4\nmatch_kind selector\n```\nUsed for implementing dynamic_action_selection (v1model)."),

        // v1model architecture: types
        "standard_metadata_t" => Some("```p4\nstruct standard_metadata_t {\n    bit<9> ingress_port;\n    bit<9> egress_spec;\n    bit<9> egress_port;\n    // ... additional fields\n}\n```\nStandard metadata structure (v1model)."),
        "standard_metadata" => Some("```p4\nstandard_metadata_t standard_metadata\n```\nStandard metadata variable passed to ingress/egress controls (v1model)."),
        "CounterType" => Some("```p4\nenum CounterType {\n    packets,\n    bytes,\n    packets_and_bytes\n}\n```\nCounter type enumeration (v1model)."),
        "MeterType" => Some("```p4\nenum MeterType {\n    packets,\n    bytes\n}\n```\nMeter type enumeration (v1model)."),

        // v1model architecture: externs
        "counter" => Some("```p4\nextern counter<I> {\n    counter(bit<32> size, CounterType type);\n    void count(in I index);\n}\n```\nCounter extern (v1model)."),
        "direct_counter" => Some("```p4\nextern direct_counter {\n    direct_counter(CounterType type);\n    void count();\n}\n```\nDirect counter associated with a table (v1model)."),
        "meter" => Some("```p4\nextern meter<I> {\n    meter(bit<32> size, MeterType type);\n    void execute_meter<T>(in I index, out T result);\n}\n```\nMeter extern (v1model)."),
        "direct_meter" => Some("```p4\nextern direct_meter<T> {\n    direct_meter(MeterType type);\n    void read(out T result);\n}\n```\nDirect meter associated with a table (v1model)."),
        "register" => Some("```p4\nextern register<T, I> {\n    register(bit<32> size);\n    void read(out T result, in I index);\n    void write(in I index, in T value);\n}\n```\nStateful register array (v1model)."),
        "mark_to_drop" => Some("```p4\nextern void mark_to_drop(inout standard_metadata_t sm)\n```\nMark the packet to be dropped (v1model)."),
        "hash" => Some("```p4\nextern void hash<O, T, D, M>(out O result, in HashAlgorithm algo, in T base, in D data, in M max)\n```\nCompute a hash function (v1model)."),
        "random" => Some("```p4\nextern void random<T>(out T result, in T lo, in T hi)\n```\nGenerate a random number in [lo, hi] (v1model)."),
        "digest" => Some("```p4\nextern void digest<T>(in bit<32> receiver, in T data)\n```\nSend a digest message to the control plane (v1model)."),
        "resubmit" => Some("```p4\nextern void resubmit<T>(in T data)\n```\nResubmit the packet to ingress (v1model)."),
        "recirculate" => Some("```p4\nextern void recirculate<T>(in T data)\n```\nRecirculate the packet (v1model)."),
        "clone" => Some("```p4\nextern void clone(in CloneType type, in bit<32> session)\n```\nClone the packet (v1model)."),
        "clone3" => Some("```p4\nextern void clone3<T>(in CloneType type, in bit<32> session, in T data)\n```\nClone the packet with metadata (v1model)."),
        "truncate" => Some("```p4\nextern void truncate(in bit<32> length)\n```\nTruncate the packet to specified length (v1model)."),
        "log_msg" => Some("```p4\nextern void log_msg(string msg)\nextern void log_msg<T>(string msg, in T data)\n```\nLog a message for debugging (v1model)."),

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

    for td in &ast.typedefs {
        if td.name == word {
            return Some(format_typedef_hover(ast, td));
        }
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

pub fn get_definition_location(ast: &AST, content: &Rope, position: Position) -> Option<Location> {
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
                if member_name == "isValid"
                    || member_name == "setValid"
                    || member_name == "setInvalid"
                {
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
                        return Some(format!(
                            "```p4\n{} {}.{}\n```\nMember of struct `{}`",
                            m.ty, type_name, m.name, type_name
                        ));
                    }
                }
            }
            if let Some(h) = ast.get_header(type_name) {
                for m in &h.members {
                    if m.name == member_name {
                        return Some(format!(
                            "```p4\n{} {}.{}\n```\nMember of header `{}`",
                            m.ty, type_name, m.name, type_name
                        ));
                    }
                }
                if member_name == "isValid"
                    || member_name == "setValid"
                    || member_name == "setInvalid"
                {
                    return Some(format!("```p4\n{}()\n```\nHeader method", member_name));
                }
            }
            if let Some(e) = ast.get_extern(type_name) {
                for m in &e.methods {
                    if m.name == member_name {
                        let params: Vec<String> = m
                            .parameters
                            .iter()
                            .map(|p| {
                                format!("{} {} {}", format_direction(&p.direction), p.ty, p.name)
                            })
                            .collect();
                        return Some(format!(
                            "```p4\n{} {}({})\n```\nMethod of extern `{}`",
                            m.return_type,
                            m.name,
                            params.join(", "),
                            type_name
                        ));
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
                        documentation: Some(Documentation::String(format!(
                            "Member of struct {}",
                            type_name
                        ))),
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
                        documentation: Some(Documentation::String(format!(
                            "Member of header {}",
                            type_name
                        ))),
                        ..Default::default()
                    });
                }
                completions.push(CompletionItem {
                    label: "isValid".to_string(),
                    kind: Some(CompletionItemKind::METHOD),
                    detail: Some("bool".to_string()),
                    insert_text: Some("isValid()".to_string()),
                    documentation: Some(Documentation::String(
                        "Check if header is valid".to_string(),
                    )),
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
                    documentation: Some(Documentation::String(
                        "Mark header as invalid".to_string(),
                    )),
                    ..Default::default()
                });
            }
            if let Some(e) = ast.get_extern(type_name) {
                for method in &e.methods {
                    let params: Vec<String> = method
                        .parameters
                        .iter()
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

fn resolve_type_for_name(
    ast: &AST,
    name: &str,
    content: &Rope,
    position: Position,
) -> Option<Type> {
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
        while prefix_start > 0
            && (is_identifier_char(chars[prefix_start - 1]) || chars[prefix_start - 1] == '.')
        {
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
    let params: Vec<String> = control
        .parameters
        .iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!(
        "```p4\ncontrol {}({})\n```",
        control.name,
        params.join(", ")
    )
}

fn format_parser_hover(parser: &Parser) -> String {
    let params: Vec<String> = parser
        .parameters
        .iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!("```p4\nparser {}({})\n```", parser.name, params.join(", "))
}

fn format_extern_hover(ext: &Extern) -> String {
    let mut result = format!("```p4\nextern {} {{\n", ext.name);
    for method in &ext.methods {
        let params: Vec<String> = method
            .parameters
            .iter()
            .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
            .collect();
        result.push_str(&format!(
            "    {} {}({});\n",
            method.return_type,
            method.name,
            params.join(", ")
        ));
    }
    result.push_str("}\n```");
    result
}

fn format_action_hover(action: &Action, control_name: &str) -> String {
    let params: Vec<String> = action
        .parameters
        .iter()
        .map(|p| format!("{} {} {}", format_direction(&p.direction), p.ty, p.name))
        .collect();
    format!(
        "```p4\n// in control {}\naction {}({})\n```",
        control_name,
        action.name,
        params.join(", ")
    )
}

fn format_typedef_hover(ast: &AST, td: &Typedef) -> String {
    let mut result = format!(
        "```p4\ntypedef {} {}\n```",
        format_type_alias(ast, &td.ty),
        td.name
    );
    if let Type::UserDefined(name) = &td.ty {
        if let Some(header) = ast.get_header(&name) {
            result.push_str("\n\n");
            result.push_str(&format_header_hover(header));
        } else if let Some(s) = ast.get_struct(&name) {
            result.push_str("\n\n");
            result.push_str(&format_struct_hover(s));
        }
    }
    result
}

fn format_type_alias(ast: &AST, ty: &Type) -> String {
    match ty {
        Type::UserDefined(name) => {
            if let Some(typedef) = ast.typedefs.iter().find(|td| td.name == *name) {
                format_type_alias(ast, &typedef.ty)
            } else {
                name.clone()
            }
        }
        _ => ty.to_string(),
    }
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
        ("abstract", "Abstract declaration modifier"),
        ("default", "Default case/action"),
        ("enum", "Define an enumeration type"),
        ("exit", "Exit the current control flow"),
        ("header_union", "Define a header union type"),
        ("list", "List type"),
        ("match_kind", "Match kind type for table keys"),
        ("switch", "Switch statement"),
        ("this", "Reference to current instance"),
        ("type", "Type alias declaration"),
        ("value_set", "Parser value set"),
        ("verify", "Verify statement in parser"),
        ("void", "Void return type"),
        ("tuple", "Tuple type"),
    ];

    for (kw, doc) in keywords {
        completions.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(doc.to_string()),
            ..Default::default()
        });
    }

    let types = ["bit", "bool", "error", "int", "varbit", "void", "tuple"];
    for ty in types {
        completions.push(CompletionItem {
            label: ty.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some("Built-in type".to_string()),
            ..Default::default()
        });
    }

    let builtins = [
        // Core library: actions
        (
            "NoAction",
            CompletionItemKind::FUNCTION,
            "Built-in action (core.p4)",
        ),
        // Core library: parser states
        (
            "accept",
            CompletionItemKind::KEYWORD,
            "Built-in parser state",
        ),
        (
            "reject",
            CompletionItemKind::KEYWORD,
            "Built-in parser state",
        ),
        // Core library: extern types
        (
            "packet_in",
            CompletionItemKind::CLASS,
            "Extern for incoming packets (core.p4)",
        ),
        (
            "packet_out",
            CompletionItemKind::CLASS,
            "Extern for outgoing packets (core.p4)",
        ),
        // Core library: types
        ("void", CompletionItemKind::TYPE_PARAMETER, "Built-in type"),
        (
            "tuple",
            CompletionItemKind::TYPE_PARAMETER,
            "Built-in tuple type",
        ),
        (
            "match_kind",
            CompletionItemKind::TYPE_PARAMETER,
            "Built-in type for match kinds",
        ),
        (
            "string",
            CompletionItemKind::TYPE_PARAMETER,
            "Built-in string type",
        ),
        // Core library: error members
        (
            "NoError",
            CompletionItemKind::ENUM_MEMBER,
            "No error (core.p4)",
        ),
        (
            "PacketTooShort",
            CompletionItemKind::ENUM_MEMBER,
            "Not enough bits in packet (core.p4)",
        ),
        (
            "NoMatch",
            CompletionItemKind::ENUM_MEMBER,
            "No match in select (core.p4)",
        ),
        (
            "StackOutOfBounds",
            CompletionItemKind::ENUM_MEMBER,
            "Invalid stack element (core.p4)",
        ),
        (
            "HeaderTooShort",
            CompletionItemKind::ENUM_MEMBER,
            "Varbit extraction overflow (core.p4)",
        ),
        (
            "ParserTimeout",
            CompletionItemKind::ENUM_MEMBER,
            "Parser timeout (core.p4)",
        ),
        (
            "ParserInvalidArgument",
            CompletionItemKind::ENUM_MEMBER,
            "Invalid parser argument (core.p4)",
        ),
        // Core library: packet_in methods
        (
            "extract",
            CompletionItemKind::METHOD,
            "Extract header from packet (core.p4)",
        ),
        (
            "lookahead",
            CompletionItemKind::METHOD,
            "Peek at packet bits (core.p4)",
        ),
        (
            "advance",
            CompletionItemKind::METHOD,
            "Advance packet cursor (core.p4)",
        ),
        // Core library: packet_out methods
        (
            "emit",
            CompletionItemKind::METHOD,
            "Emit header to packet (core.p4)",
        ),
        // Core library: header methods
        (
            "isValid",
            CompletionItemKind::METHOD,
            "Check if header is valid",
        ),
        (
            "setValid",
            CompletionItemKind::METHOD,
            "Mark header as valid",
        ),
        (
            "setInvalid",
            CompletionItemKind::METHOD,
            "Mark header as invalid",
        ),
        // Core library: functions
        (
            "static_assert",
            CompletionItemKind::FUNCTION,
            "Compile-time assertion (core.p4)",
        ),
        // v1model architecture: types
        (
            "standard_metadata_t",
            CompletionItemKind::STRUCT,
            "Standard metadata structure (v1model)",
        ),
        (
            "standard_metadata",
            CompletionItemKind::VARIABLE,
            "Standard metadata variable (v1model)",
        ),
        (
            "CounterType",
            CompletionItemKind::ENUM,
            "Counter type enum (v1model)",
        ),
        (
            "MeterType",
            CompletionItemKind::ENUM,
            "Meter type enum (v1model)",
        ),
        (
            "HashAlgorithm",
            CompletionItemKind::ENUM,
            "Hash algorithm enum (v1model)",
        ),
        (
            "CloneType",
            CompletionItemKind::ENUM,
            "Clone type enum (v1model)",
        ),
        // v1model architecture: match_kind extensions
        (
            "optional",
            CompletionItemKind::ENUM_MEMBER,
            "Optional match kind (v1model)",
        ),
        (
            "selector",
            CompletionItemKind::ENUM_MEMBER,
            "Selector match kind (v1model)",
        ),
        // v1model architecture: enum values
        (
            "packets",
            CompletionItemKind::ENUM_MEMBER,
            "Count packets (v1model)",
        ),
        (
            "bytes",
            CompletionItemKind::ENUM_MEMBER,
            "Count bytes (v1model)",
        ),
        (
            "packets_and_bytes",
            CompletionItemKind::ENUM_MEMBER,
            "Count both (v1model)",
        ),
        // v1model architecture: externs
        (
            "counter",
            CompletionItemKind::CLASS,
            "Counter extern (v1model)",
        ),
        (
            "direct_counter",
            CompletionItemKind::CLASS,
            "Direct counter extern (v1model)",
        ),
        ("meter", CompletionItemKind::CLASS, "Meter extern (v1model)"),
        (
            "direct_meter",
            CompletionItemKind::CLASS,
            "Direct meter extern (v1model)",
        ),
        (
            "register",
            CompletionItemKind::CLASS,
            "Register extern (v1model)",
        ),
        (
            "action_profile",
            CompletionItemKind::CLASS,
            "Action profile extern (v1model)",
        ),
        (
            "action_selector",
            CompletionItemKind::CLASS,
            "Action selector extern (v1model)",
        ),
        // v1model architecture: functions
        (
            "mark_to_drop",
            CompletionItemKind::FUNCTION,
            "Mark packet to drop (v1model)",
        ),
        (
            "hash",
            CompletionItemKind::FUNCTION,
            "Compute hash (v1model)",
        ),
        (
            "random",
            CompletionItemKind::FUNCTION,
            "Generate random number (v1model)",
        ),
        (
            "digest",
            CompletionItemKind::FUNCTION,
            "Send digest to control plane (v1model)",
        ),
        (
            "resubmit",
            CompletionItemKind::FUNCTION,
            "Resubmit packet (v1model)",
        ),
        (
            "recirculate",
            CompletionItemKind::FUNCTION,
            "Recirculate packet (v1model)",
        ),
        (
            "clone",
            CompletionItemKind::FUNCTION,
            "Clone packet (v1model)",
        ),
        (
            "clone3",
            CompletionItemKind::FUNCTION,
            "Clone packet with metadata (v1model)",
        ),
        (
            "truncate",
            CompletionItemKind::FUNCTION,
            "Truncate packet (v1model)",
        ),
        (
            "log_msg",
            CompletionItemKind::FUNCTION,
            "Log debug message (v1model)",
        ),
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
