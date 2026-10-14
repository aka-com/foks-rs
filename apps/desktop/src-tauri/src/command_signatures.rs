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

const REQUIRED: &str = "required";
const OPTIONAL: &str = "optional";

#[derive(Debug, PartialEq)]
struct Command {
    /// The module path from the crate root, joined with `::`.
    module: String,
    /// camelCase key to [`REQUIRED`] or [`OPTIONAL`].
    arguments: BTreeMap<String, &'static str>,
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
    from_tauri
        && segments
            .last()
            .is_some_and(|segment| INJECTED.iter().any(|kind| segment.ident == kind))
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
    for input in &function.sig.inputs {
        let syn::FnArg::Typed(argument) = input else {
            panic!("{module}::{name} takes self; commands are free functions");
        };
        let path = type_path(&argument.ty);
        if path.is_some_and(is_injected) {
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
    Command {
        module: module.to_owned(),
        arguments,
    }
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

#[test]
fn generate_handler_registers_exactly_the_defined_commands() {
    let commands = read_commands();
    let mut handlers = Handlers::default();
    handlers.visit_file(&parse(&source_root().join("lib.rs")));
    let [registered] = handlers.0.as_slice() else {
        panic!("lib.rs has exactly one generate_handler! invocation");
    };

    let mut seen = BTreeSet::new();
    for path in registered {
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
}
