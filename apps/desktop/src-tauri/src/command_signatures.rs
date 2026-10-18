//! The command names and argument keys the renderer bridge is checked against.
//!
//! Tauri binds command arguments by name. Each parameter that the runtime does
//! not inject (the app handle, the calling webview or window, managed state) is
//! read from the camelCase form of its name in the invoke payload. An
//! `Option<_>` parameter may be omitted; any other missing key fails the call
//! at runtime with "missing required key". Nothing at compile time ties the
//! string literals in `src/bridge` to these signatures, so this test reads them
//! from the `#[tauri::command]` functions, checks that they are exactly the
//! functions `generate_handler!` registers, and compares the result with
//! `command-signatures.json`. `tests/command-contract.test.ts` checks every
//! bridge call site against that file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

const MANIFEST: &str = "command-signatures.json";

/// Set to rewrite `command-signatures.json` instead of comparing with it.
const WRITE_VARIABLE: &str = "FOKS_WRITE_COMMAND_SIGNATURES";

const CHECK_COMMAND: &str = "cargo test --locked -p foks-desktop-app --lib command_signatures";

/// Parameter types that Tauri supplies from the invocation instead of the
/// payload. A parameter is injected when its type path is one of these names,
/// either bare (imported from `tauri`) or under `tauri::`.
/// `crate_items_do_not_reuse_injected_names` keeps a bare name unambiguous.
/// `Channel` is not among them: the renderer passes it under its key like any
/// other argument.
const INJECTED: [&str; 8] = [
    "AppHandle",
    "Webview",
    "WebviewWindow",
    "Window",
    "State",
    "Request",
    "CommandScope",
    "GlobalScope",
];

/// Crate-defined parameter types that Tauri builds from the invocation through
/// their `CommandArg` implementation instead of reading a payload key.
/// Commands name them by a crate path such as `super::deadline::CommandState`,
/// so they are matched by the last path segment from any path; a crate item
/// of the same name is the extractor itself.
const CRATE_INJECTED: [&str; 1] = ["CommandState"];

/// The extractor whose construction applies `commands::deadline::command_class`.
const DEADLINE_EXTRACTOR: &str = "CommandState";

const REQUIRED: &str = "required";
const OPTIONAL: &str = "optional";

#[derive(Debug, PartialEq)]
struct Command {
    /// The module path from the crate root, joined with `::`.
    module: String,
    /// camelCase key to [`REQUIRED`] or [`OPTIONAL`].
    arguments: BTreeMap<String, &'static str>,
    /// The path of every function the body calls, segments joined with `::`,
    /// for [`every_command_checks_the_main_window`].
    calls: BTreeSet<String>,
    /// Whether a parameter is [`DEADLINE_EXTRACTOR`], so that invoking the
    /// command consults its deadline policy.
    deadline_policy: bool,
}

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn parse(path: &Path) -> syn::File {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
    syn::parse_file(&text).unwrap_or_else(|error| panic!("{} parses: {error}", path.display()))
}

fn is_test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| match &attribute.meta {
        syn::Meta::List(list) => list.path.is_ident("cfg") && list.tokens.to_string() == "test",
        _ => false,
    })
}

fn is_command(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        let segments = &attribute.path().segments;
        let command =
            segments.len() == 2 && segments[0].ident == "tauri" && segments[1].ident == "command";
        assert!(
            !command || matches!(attribute.meta, syn::Meta::Path(_)),
            "#[tauri::command] options such as rename_all change the argument keys, and \
             command_signatures.rs does not read them; add support for the option before \
             using it"
        );
        command
    })
}

fn type_path(ty: &syn::Type) -> Option<&syn::Path> {
    match ty {
        syn::Type::Path(path) if path.qself.is_none() => Some(&path.path),
        _ => None,
    }
}

/// Whether Tauri supplies a parameter of this type instead of the payload.
fn is_injected(path: &syn::Path) -> bool {
    let segments = &path.segments;
    let from_tauri = segments.len() == 1 || segments[0].ident == "tauri";
    let Some(last) = segments.last() else {
        return false;
    };
    (from_tauri && INJECTED.iter().any(|kind| last.ident == kind))
        || CRATE_INJECTED.iter().any(|kind| last.ident == kind)
}

/// Tauri's default argument rename: `store_id` is sent as `storeId`.
fn camel_case(name: &str) -> String {
    let mut key = String::new();
    for (index, word) in name.split('_').filter(|word| !word.is_empty()).enumerate() {
        let mut characters = word.chars();
        if index > 0 {
            key.extend(characters.next().into_iter().flat_map(char::to_uppercase));
        }
        key.push_str(characters.as_str());
    }
    key
}

fn signature(function: &syn::ItemFn, module: &str) -> Command {
    let name = &function.sig.ident;
    let mut arguments = BTreeMap::new();
    let mut deadline_policy = false;
    for input in &function.sig.inputs {
        let syn::FnArg::Typed(argument) = input else {
            panic!("{module}::{name} takes self; commands are free functions");
        };
        let path = type_path(&argument.ty);
        if path.is_some_and(is_injected) {
            deadline_policy |= path
                .and_then(|path| path.segments.last())
                .is_some_and(|segment| segment.ident == DEADLINE_EXTRACTOR);
            continue;
        }
        let syn::Pat::Ident(binding) = &*argument.pat else {
            panic!("{module}::{name} destructures a parameter; Tauri keys arguments by name");
        };
        let key = camel_case(&binding.ident.to_string());
        let presence = if path
            .and_then(|path| path.segments.last())
            .is_some_and(|segment| segment.ident == "Option")
        {
            OPTIONAL
        } else {
            REQUIRED
        };
        assert!(
            arguments.insert(key.clone(), presence).is_none(),
            "{module}::{name} has two parameters keyed {key}"
        );
    }
    let mut calls = Calls::default();
    calls.visit_block(&function.block);
    Command {
        module: module.to_owned(),
        arguments,
        calls: calls.0,
        deadline_policy,
    }
}

/// String literals in the patterns visited.
#[derive(Default)]
struct PatternStrings(BTreeSet<String>);

impl<'ast> Visit<'ast> for PatternStrings {
    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        self.0.insert(literal.value());
    }
}

/// The command names that `commands::deadline::command_class` gives a policy:
/// the string patterns of its match arms.
fn deadline_policy_names() -> BTreeSet<String> {
    let file = parse(&source_root().join("commands").join("deadline.rs"));
    let function = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == "command_class" => Some(function),
            _ => None,
        })
        .expect("commands/deadline.rs defines command_class");
    struct Arms(PatternStrings);
    impl<'ast> Visit<'ast> for Arms {
        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            self.0.visit_pat(&arm.pat);
            syn::visit::visit_arm(self, arm);
        }
    }
    let mut arms = Arms(PatternStrings::default());
    arms.visit_item_fn(function);
    arms.0 .0
}

/// The functions a body calls by path, segments joined with `::`.
#[derive(Default)]
struct Calls(BTreeSet<String>);

impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            let segments: Vec<String> = path
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            self.0.insert(segments.join("::"));
        }
        syn::visit::visit_expr_call(self, call);
    }
}

/// The commands that call `require_main_window`, directly or through
/// another command they delegate to. A delegating call names the command
/// either bare, from the command's own module, or by its full
/// `crate::` path; a call to a same-named function elsewhere does not count.
fn commands_checking_the_main_window(commands: &BTreeMap<String, Command>) -> BTreeSet<String> {
    let mut checked: BTreeSet<String> = commands
        .iter()
        .filter(|(_, command)| {
            command
                .calls
                .iter()
                .any(|call| call.rsplit("::").next() == Some("require_main_window"))
        })
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let delegating: Vec<String> = commands
            .iter()
            .filter(|(name, command)| {
                !checked.contains(*name)
                    && checked.iter().any(|callee| {
                        command
                            .calls
                            .iter()
                            .any(|call| calls_command(command, call, callee, &commands[callee]))
                    })
            })
            .map(|(name, _)| name.clone())
            .collect();
        if delegating.is_empty() {
            return checked;
        }
        checked.extend(delegating);
    }
}

/// Whether `call`, made in the body of `caller`, names the command `name`
/// defined by `callee`: bare from the same module, or by its `crate::` path.
fn calls_command(caller: &Command, call: &str, name: &str, callee: &Command) -> bool {
    let path = std::iter::once("crate")
        .chain(
            callee
                .module
                .split("::")
                .filter(|segment| !segment.is_empty()),
        )
        .chain(std::iter::once(name))
        .collect::<Vec<_>>()
        .join("::");
    (call == name && caller.module == callee.module) || call == path
}

/// Record the commands in `items`, following `mod` declarations the way the
/// compiler does. `directory` holds the files of this module's children.
fn collect(
    items: &[syn::Item],
    directory: &Path,
    module: &[String],
    commands: &mut BTreeMap<String, Command>,
) {
    for item in items {
        match item {
            syn::Item::Fn(function) if is_command(&function.attrs) => {
                let name = function.sig.ident.to_string();
                let command = signature(function, &module.join("::"));
                // A command defined once per platform must agree with itself.
                if let Some(existing) = commands.get(&name) {
                    assert_eq!(existing, &command, "{name} is defined twice, differently");
                }
                commands.insert(name, command);
            }
            syn::Item::Mod(child) if !is_test_only(&child.attrs) => {
                let name = child.ident.to_string();
                let nested = directory.join(&name);
                let mut path = module.to_vec();
                path.push(name.clone());
                if let Some((_, items)) = &child.content {
                    collect(items, &nested, &path, commands);
                    continue;
                }
                let file = [directory.join(format!("{name}.rs")), nested.join("mod.rs")]
                    .into_iter()
                    .find(|candidate| candidate.is_file())
                    .unwrap_or_else(|| panic!("no source file for module {}", path.join("::")));
                collect(&parse(&file).items, &nested, &path, commands);
            }
            _ => {}
        }
    }
}

/// The handler paths listed in `generate_handler!`, joined with `::`.
#[derive(Default)]
struct Handlers(Vec<Vec<String>>);

impl<'ast> Visit<'ast> for Handlers {
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "generate_handler")
        {
            let paths = invocation
                .parse_body_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .expect("generate_handler! lists command paths");
            self.0.push(
                paths
                    .iter()
                    .map(|path| {
                        path.segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect::<Vec<_>>()
                            .join("::")
                    })
                    .collect(),
            );
        }
        syn::visit::visit_macro(self, invocation);
    }
}

/// Crate items and imports that bind one of the [`INJECTED`] names to something
/// other than the `tauri` type, as `path: name`.
struct LocalInjectedNames<'a> {
    path: &'a Path,
    found: Vec<String>,
}

impl LocalInjectedNames<'_> {
    fn check(&mut self, ident: &syn::Ident) {
        if INJECTED.iter().any(|kind| ident == kind) {
            self.found.push(format!("{}: {ident}", self.path.display()));
        }
    }

    fn check_use(&mut self, tree: &syn::UseTree, from_tauri: bool) {
        match tree {
            syn::UseTree::Path(path) => self.check_use(&path.tree, from_tauri),
            syn::UseTree::Name(name) if !from_tauri => self.check(&name.ident),
            syn::UseTree::Rename(rename) if !from_tauri || rename.ident != rename.rename => {
                self.check(&rename.rename)
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.check_use(tree, from_tauri);
                }
            }
            _ => {}
        }
    }
}

impl<'ast> Visit<'ast> for LocalInjectedNames<'_> {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.check(&item.ident);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.check(&item.ident);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.check(&item.ident);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.check(&item.ident);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let from_tauri = matches!(&item.tree, syn::UseTree::Path(path) if path.ident == "tauri");
        self.check_use(&item.tree, from_tauri);
    }
}

fn source_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", directory.display()));
    for entry in entries {
        let path = entry.expect("directory entries are readable").path();
        if path.is_dir() {
            source_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn manifest(commands: &BTreeMap<String, Command>) -> serde_json::Value {
    serde_json::json!({
        "sourceOfTruth": concat!(
            "The #[tauri::command] functions under apps/desktop/src-tauri/src that ",
            "generate_handler! registers. Each command maps the camelCase key of every ",
            "parameter Tauri does not inject to required, or to optional for an Option ",
            "parameter.",
        ),
        "checkCommand": CHECK_COMMAND,
        "writeCommand": format!("{WRITE_VARIABLE}=1 {CHECK_COMMAND}"),
        "commands": commands
            .iter()
            .map(|(name, command)| (name.clone(), serde_json::json!(command.arguments)))
            .collect::<serde_json::Map<_, _>>(),
    })
}

fn read_commands() -> BTreeMap<String, Command> {
    let root = source_root();
    let mut commands = BTreeMap::new();
    collect(
        &parse(&root.join("lib.rs")).items,
        &root,
        &[],
        &mut commands,
    );
    commands
}

/// The handler paths of the single `generate_handler!` in `lib.rs`, in order.
fn registered_paths() -> Vec<String> {
    let mut handlers = Handlers::default();
    handlers.visit_file(&parse(&source_root().join("lib.rs")));
    let Ok([registered]) = <[Vec<String>; 1]>::try_from(handlers.0) else {
        panic!("lib.rs has exactly one generate_handler! invocation");
    };
    registered
}

/// The names the renderer invokes: the last segment of each registered path.
pub(crate) fn registered_command_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for path in registered_paths() {
        let name = path.rsplit("::").next().unwrap_or(&path).to_owned();
        assert!(
            names.insert(name),
            "two registered commands are named like {path}"
        );
    }
    names
}

#[test]
fn generate_handler_registers_exactly_the_defined_commands() {
    let commands = read_commands();
    let registered = registered_paths();

    let mut seen = BTreeSet::new();
    for path in &registered {
        assert!(seen.insert(path), "{path} is registered twice");
    }
    let defined: BTreeSet<String> = commands
        .iter()
        .map(|(name, command)| match command.module.as_str() {
            "" => name.clone(),
            module => format!("{module}::{name}"),
        })
        .collect();
    let registered: BTreeSet<String> = registered.iter().cloned().collect();
    assert_eq!(
        defined.difference(&registered).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "#[tauri::command] functions missing from generate_handler!"
    );
    assert_eq!(
        registered.difference(&defined).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "generate_handler! entries with no #[tauri::command] function"
    );
}

/// Tauri's app ACL already limits every command to the `main` window on the
/// app's own origin; each command repeats the label check so that a capability
/// granted too widely does not by itself open a command to another window.
#[test]
fn every_command_checks_the_main_window() {
    let commands = read_commands();
    let checked = commands_checking_the_main_window(&commands);
    let unchecked: Vec<&String> = commands
        .keys()
        .filter(|name| !checked.contains(*name))
        .collect();
    assert_eq!(
        unchecked,
        Vec::<&String>::new(),
        "commands that do not call require_main_window"
    );
}

/// `CommandState` fails closed for a command without a deadline policy, so
/// every command that takes it needs one; a policy for a command that does
/// not take it is never consulted.
#[test]
fn deadline_policies_name_exactly_the_commands_that_take_command_state() {
    let admitted: BTreeSet<String> = read_commands()
        .into_iter()
        .filter(|(_, command)| command.deadline_policy)
        .map(|(name, _)| name)
        .collect();
    let policies = deadline_policy_names();
    assert_eq!(
        admitted.difference(&policies).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "commands taking CommandState with no command_class policy"
    );
    assert_eq!(
        policies.difference(&admitted).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "command_class policies for commands that do not take CommandState"
    );
}

#[test]
fn command_signatures_match_the_manifest() {
    let expected = manifest(&read_commands());
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(MANIFEST);
    if std::env::var_os(WRITE_VARIABLE).is_some() {
        let text = serde_json::to_string_pretty(&expected).expect("the manifest serializes");
        std::fs::write(&path, format!("{text}\n"))
            .unwrap_or_else(|error| panic!("{} is writable: {error}", path.display()));
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is readable: {error}", path.display()));
    let actual: serde_json::Value = serde_json::from_str(&text).expect("the manifest is JSON");
    assert!(
        actual == expected,
        "{MANIFEST} is stale; rerun with {WRITE_VARIABLE}=1 and check the bridge with \
         tests/command-contract.test.ts"
    );
}

#[test]
fn crate_items_do_not_reuse_injected_names() {
    // A bare parameter type such as `Request` counts as injected only while
    // nothing in the crate binds that name to a payload type.
    let mut files = Vec::new();
    source_files(&source_root(), &mut files);
    files.sort();
    let mut found = Vec::new();
    for file in &files {
        let mut names = LocalInjectedNames {
            path: file.strip_prefix(source_root()).unwrap_or(file),
            found: Vec::new(),
        };
        names.visit_file(&parse(file));
        found.append(&mut names.found);
    }
    assert_eq!(found, Vec::<String>::new());
}

#[test]
fn local_names_report_payload_types_with_injected_names() {
    let file: syn::File = syn::parse_quote! {
        use tauri::{AppHandle, Manager as _, State};
        use foks_agent_proto::{Request, Response};
        use tauri::Window as Frame;
        use tauri::Webview as Webview;
        type GlobalScope = u8;
        struct Commands;
    };
    let mut names = LocalInjectedNames {
        path: Path::new("example.rs"),
        found: Vec::new(),
    };
    names.visit_file(&file);
    assert_eq!(
        names.found,
        ["example.rs: Request", "example.rs: GlobalScope"]
    );
}

#[test]
fn argument_keys_use_tauris_camel_case() {
    assert_eq!(camel_case("store_id"), "storeId");
    assert_eq!(camel_case("pq_slot"), "pqSlot");
    assert_eq!(camel_case("profile"), "profile");
    assert_eq!(camel_case("_unused"), "unused");
}

#[test]
fn signatures_skip_injected_parameters_and_mark_options() {
    let function: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub async fn example(
            app: tauri::AppHandle,
            webview: tauri::Webview,
            state: State<'_, AppState>,
            command: super::deadline::CommandState,
            window: tauri::window::Window,
            store_id: String,
            on_partial: tauri::ipc::Channel<CatalogDto>,
            read_role: Option<RoleInput>,
            request: foks_agent_proto::Request,
        ) -> Result<(), AgentError> {
            Ok(())
        }
    };
    assert!(is_command(&function.attrs));
    assert_eq!(
        signature(&function, "commands::vault").arguments,
        BTreeMap::from([
            ("onPartial".to_owned(), REQUIRED),
            ("readRole".to_owned(), OPTIONAL),
            ("request".to_owned(), REQUIRED),
            ("storeId".to_owned(), REQUIRED),
        ])
    );
    assert!(signature(&function, "commands::vault").deadline_policy);
}

#[test]
fn a_command_checks_the_main_window_directly_or_through_a_checked_command() {
    let direct: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub fn direct(webview: tauri::Webview) -> Result<(), AgentError> {
            crate::commands::require_main_window(&webview)?;
            Ok(())
        }
    };
    let delegating: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub async fn delegating(webview: tauri::Webview) -> Result<(), AgentError> {
            direct(webview)
        }
    };
    let by_path: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub async fn by_path(webview: tauri::Webview) -> Result<(), AgentError> {
            crate::applock::direct(webview)
        }
    };
    let unchecked: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub fn unchecked(window: tauri::WebviewWindow) -> Result<(), AgentError> {
            helper(window.label());
            Ok(())
        }
    };
    // `direct` here is another module's function, not the checked command.
    let other_module: syn::ItemFn = syn::parse_quote! {
        #[tauri::command]
        pub async fn other_module(webview: tauri::Webview) -> Result<(), AgentError> {
            direct(webview);
            client::direct(webview)
        }
    };
    let commands = BTreeMap::from([
        ("direct".to_owned(), signature(&direct, "applock")),
        ("delegating".to_owned(), signature(&delegating, "applock")),
        ("by_path".to_owned(), signature(&by_path, "vault")),
        ("unchecked".to_owned(), signature(&unchecked, "applock")),
        ("other_module".to_owned(), signature(&other_module, "vault")),
    ]);
    assert_eq!(
        commands_checking_the_main_window(&commands),
        BTreeSet::from([
            "by_path".to_owned(),
            "delegating".to_owned(),
            "direct".to_owned()
        ])
    );
}
