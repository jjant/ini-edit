//! The crate's thread-safety promises, pinned at compile time and used across threads.
//!
//! The first test names the auto traits that each public type has, and the `compile_fail`
//! doctests in `src/auto_traits.rs` the ones it lacks, so a change that adds or removes one fails
//! a test; the crate documentation's Threads section lists them. The tests then rely on them: an
//! editor moves to another thread and is edited there, threads share one editor and one parse,
//! threads walk a snapshot of a tree while another thread edits the editor that shares its nodes,
//! handles move between threads, and threads edit editors of their own. Every result is compared
//! with the same work done in one thread. CI also runs this binary under the thread sanitizer
//! (`-Zsanitizer=thread`, see TESTING.md), which reports any data race between the threads, in
//! this crate, in rowan or in the standard library.

use std::panic::{RefUnwindSafe, UnwindSafe};
use std::sync::Barrier;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use ini_edit::ast::{AstNode, Entry, File};
use ini_edit::editor::{
    EditError, EditOptions, Editor, EntryEditor, SectionEditor, SeparatorSpacing, Whitespace,
};
use ini_edit::lexer::Token;
use ini_edit::{IniLang, Parse, ParseError, ParseOptions, SyntaxKind, SyntaxNode, parse_with};

/// The traits of a type that holds no cursor and no `&mut`. A parse and an editor hold rowan's
/// green tree, whose nodes count their references atomically; a lexer token borrows its text as a
/// `&str`.
fn holds_no_cursor_and_no_mut<T: Send + Sync + UnwindSafe + RefUnwindSafe>() {}

/// The traits of an editor's handles. A handle holds a `&mut Editor`, which is not `UnwindSafe`.
fn borrows_an_editor<T: Send + Sync + RefUnwindSafe>() {}

/// Checked when it compiles: a type that lacks one of its traits fails the build here.
#[test]
fn every_public_type_has_its_auto_traits() {
    holds_no_cursor_and_no_mut::<Parse>();
    holds_no_cursor_and_no_mut::<ParseError>();
    holds_no_cursor_and_no_mut::<ParseOptions>();
    holds_no_cursor_and_no_mut::<Editor>();
    holds_no_cursor_and_no_mut::<EditOptions>();
    holds_no_cursor_and_no_mut::<SeparatorSpacing>();
    holds_no_cursor_and_no_mut::<Whitespace>();
    holds_no_cursor_and_no_mut::<EditError>();
    holds_no_cursor_and_no_mut::<SyntaxKind>();
    holds_no_cursor_and_no_mut::<IniLang>();
    holds_no_cursor_and_no_mut::<Token<'static>>();
    borrows_an_editor::<SectionEditor<'static>>();
    borrows_an_editor::<EntryEditor<'static>>();
}

/// The threads that each test starts, besides a writer.
const THREADS: usize = 8;

/// How many times each reader repeats its reads.
const ROUNDS: usize = 64;

/// The most edits that the writer of
/// [`threads_walk_a_snapshot_while_another_thread_edits_the_editor`] makes while it waits for the
/// readers, far more than they take.
const WAITING_EDITS_MAX: usize = 100_000;

/// A preamble, a continued value, an inline comment, a bare key, an unclosed header, a comment
/// and CRLF line breaks.
const SOURCE: &str = "root = 1\r\n[server]\r\nhost = a \\\r\n  b\r\nport = 80 ; web\r\n\
    quick\r\n\r\n[db]\r\nurl = x\r\n[unclosed\r\n; the end\r\n";

fn parse_options() -> ParseOptions {
    ParseOptions {
        allow_no_value: true,
        inline_comments: true,
    }
}

fn edit_options() -> EditOptions {
    let separator_spacing = SeparatorSpacing::exact("\t", " ").expect("spaces and tabs");
    EditOptions { separator_spacing }
}

fn open() -> Editor {
    Editor::new(SOURCE, &parse_options(), &edit_options()).expect("the source fits")
}

/// The edits that the tests make, in one thread or another.
fn edit(editor: &mut Editor) -> Result<(), EditError> {
    let mut server = editor.section_or_insert("server")?;
    server.set("port", "8080")?;
    server.append_entry("timeout", "30")?;
    server.entry("host").expect("host exists").set_value("c")?;
    editor.preamble().set("root", "2")?;
    let renamed = editor
        .section("db")
        .expect("db exists")
        .rename_key("url", "dsn")?;
    assert!(renamed);
    editor
        .section_or_insert("cache")?
        .append_entry("size", "64")
}

/// The edits that follow [`edit`]: a removal, an insertion and a removed section.
fn edit_again(editor: &mut Editor) -> Result<(), EditError> {
    let mut server = editor.section("server").expect("server exists");
    let removed = server.remove_entry("quick")?;
    assert!(removed);
    server.insert_entry(0, "first", "yes")?;
    editor.section("cache").expect("cache exists").remove();
    Ok(())
}

/// Edits through a section handle and an entry handle.
fn edit_server(server: &mut SectionEditor<'_>) -> Result<(), EditError> {
    server.set("port", "8080")?;
    let mut host = server.entry("host").expect("host exists");
    host.set_key("hostname")?;
    host.set_value("c")
}

/// A tree as a reader sees it: the preamble, named `None`, then each section, with each entry's
/// key, value and inline comment.
type Tree = Vec<(Option<String>, Vec<[Option<String>; 3]>)>;

fn read_tree(file: &File) -> Tree {
    let entry = |entry: Entry| [entry.key(), entry.value(), entry.inline_comment()];
    let preamble = (None, file.preamble_entries().map(entry).collect());
    let sections = file
        .sections()
        .map(|section| (section.name(), section.entries().map(entry).collect()));
    std::iter::once(preamble).chain(sections).collect()
}

/// What a reader of an editor sees.
#[derive(Debug, PartialEq)]
struct EditorView {
    text: String,
    section_count: usize,
    parse_options: ParseOptions,
    edit_options: EditOptions,
    tree: Tree,
}

fn read_editor(editor: &Editor) -> EditorView {
    EditorView {
        text: editor.finish(),
        section_count: editor.section_count(),
        parse_options: editor.parse_options().clone(),
        edit_options: editor.edit_options().clone(),
        tree: read_tree(&editor.file()),
    }
}

/// What a reader of a parse sees: its text, its errors and its tree.
#[derive(Debug, PartialEq)]
struct ParseView {
    text: String,
    errors: Vec<(String, usize)>,
    tree: Tree,
}

fn read_parse(parse: &Parse) -> ParseView {
    let root = parse.syntax();
    let errors = parse.errors().iter();
    ParseView {
        text: root.text().to_string(),
        errors: errors
            .map(|error| (error.message.clone(), error.offset))
            .collect(),
        tree: read_tree(&File::cast(root).expect("the root is a file")),
    }
}

/// A function that makes edits, such as [`edit`].
type Edits = fn(&mut Editor) -> Result<(), EditError>;

/// The editor after `edits`, made in this thread.
fn edited(edits: &[Edits]) -> EditorView {
    let mut editor = open();
    for edit in edits {
        edit(&mut editor).expect("the edits succeed");
    }
    read_editor(&editor)
}

#[test]
fn an_editor_moves_to_another_thread_and_is_edited_there() {
    let mut editor = open();
    let worker = thread::spawn(move || edit(&mut editor).map(|()| editor));
    let mut editor = worker.join().expect("no panic").expect("the edits succeed");
    assert_eq!(read_editor(&editor), edited(&[edit]));
    // Back in this thread, the editor edits on, and a third thread frees its tree.
    edit_again(&mut editor).expect("the edits succeed");
    assert_eq!(read_editor(&editor), edited(&[edit, edit_again]));
    thread::spawn(move || drop(editor))
        .join()
        .expect("no panic");
}

#[test]
fn threads_share_an_editor_and_a_parse() {
    let mut editor = open();
    let parse = parse_with(SOURCE, &parse_options());
    let editor_view = read_editor(&editor);
    let parse_view = read_parse(&parse);
    assert_eq!(parse_view.text, SOURCE);
    assert_eq!(parse_view.tree, editor_view.tree);
    assert_eq!(parse_view.errors.len(), 1);
    let barrier = Barrier::new(THREADS);
    thread::scope(|scope| {
        let readers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    // All at once, so that their updates of the shared reference counts overlap.
                    barrier.wait();
                    for _ in 0..ROUNDS {
                        assert_eq!(read_editor(&editor), editor_view);
                        // A clone shares the green tree, so its reference count changes in every
                        // thread.
                        let copy = parse.clone();
                        assert_eq!(read_parse(&copy), parse_view);
                        // The editor's text, parsed again with its options, has the same tree.
                        let again = parse_with(&editor.finish(), editor.parse_options());
                        assert_eq!(again.green(), parse.green());
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().expect("no panic");
        }
    });
    // Once the threads end, the editor is this thread's to edit again.
    edit(&mut editor).expect("the edits succeed");
    assert_eq!(read_editor(&editor), edited(&[edit]));
    assert_eq!(read_parse(&parse), parse_view);
}

/// Counts a reader as done when it drops, even if the reader panics, so that the writer stops.
struct Done<'count>(&'count AtomicUsize);

impl Drop for Done<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Release);
    }
}

/// Edits `editor` until all [`THREADS`] readers are `done`, and returns how many edits that took.
/// Setting the port to the value that [`edit`] gave it leaves the text as it is.
fn edit_until_done(editor: &mut Editor, done: &AtomicUsize) -> Result<usize, EditError> {
    for edits in 0..WAITING_EDITS_MAX {
        if done.load(Ordering::Acquire) == THREADS {
            return Ok(edits);
        }
        editor
            .section("server")
            .expect("server exists")
            .set("port", "8080")?;
    }
    panic!("the readers are not done after {WAITING_EDITS_MAX} edits");
}

#[test]
fn threads_walk_a_snapshot_while_another_thread_edits_the_editor() {
    let mut editor = open();
    let before = read_editor(&editor);
    // A green tree, unlike a cursor, crosses threads. The edits rebuild the root and the
    // sections they change; the other nodes stay shared between this tree and the editor's.
    let snapshot = editor.file().syntax().green().to_owned();
    assert_eq!(snapshot.to_string(), SOURCE);
    let (barrier, done) = (Barrier::new(THREADS + 1), AtomicUsize::new(0));
    let (before, barrier, done) = (&before, &barrier, &done);
    thread::scope(|scope| {
        let readers: Vec<_> = (0..THREADS)
            .map(|_| {
                let snapshot = snapshot.clone();
                scope.spawn(move || {
                    let done = Done(done);
                    barrier.wait();
                    for _ in 0..ROUNDS {
                        let root = SyntaxNode::new_root(snapshot.clone());
                        assert_eq!(root.text().to_string(), before.text);
                        let file = File::cast(root).expect("the root is a file");
                        assert_eq!(read_tree(&file), before.tree);
                    }
                    drop(snapshot);
                    drop(done);
                })
            })
            .collect();
        // The editor and the readers now own the old tree, and the editor lets go at its first
        // edit. The writer edits on until the readers are done, so the last reader frees the old
        // tree, and releases the nodes it shares with the editor's tree, while the writer
        // rebuilds trees from those nodes.
        drop(snapshot);
        let writer = scope.spawn(|| {
            barrier.wait();
            edit(&mut editor)?;
            edit_until_done(&mut editor, done)
        });
        writer.join().expect("no panic").expect("the edits succeed");
        for reader in readers {
            reader.join().expect("no panic");
        }
    });
    assert_eq!(read_editor(&editor), edited(&[edit]));
}

#[test]
fn a_section_handle_moves_to_another_thread_and_threads_share_it() {
    let mut editor = open();
    let mut server = editor.section("server").expect("server exists");
    let reads = (server.name(), server.entry_count(), SOURCE.to_owned());
    let barrier = Barrier::new(THREADS);
    thread::scope(|scope| {
        let (server, reads, barrier) = (&server, &reads, &barrier);
        let readers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..ROUNDS {
                        let text = server.editor().finish();
                        assert_eq!(&(server.name(), server.entry_count(), text), reads);
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().expect("no panic");
        }
    });
    // The handle moves to a thread, which edits through it and hands it back.
    let server = thread::scope(|scope| {
        let worker = scope.spawn(move || edit_server(&mut server).map(|()| server));
        worker.join().expect("no panic").expect("the edits succeed")
    });
    let mut expected = open();
    edit_server(&mut expected.section("server").expect("server exists")).expect("it succeeds");
    assert_eq!(read_editor(server.editor()), read_editor(&expected));
}

/// What a reader of an entry handle sees: its key, value and inline comment.
fn read_entry(entry: &EntryEditor<'_>) -> [Option<String>; 3] {
    [Some(entry.key()), entry.value(), entry.inline_comment()]
}

#[test]
fn an_entry_handle_moves_to_another_thread_and_threads_share_it() {
    let mut expected = open();
    let reads = {
        let mut server = expected.section("server").expect("server exists");
        let mut port = server.entry("port").expect("port exists");
        port.set_value("9090").expect("the edit succeeds");
        read_entry(&port)
    };
    let mut editor = open();
    let mut server = editor.section("server").expect("server exists");
    let mut port = server.entry("port").expect("port exists");
    // The handle moves to a thread, which edits through it and hands it back.
    port = thread::scope(|scope| {
        let worker = scope.spawn(move || port.set_value("9090").map(|()| port));
        worker.join().expect("no panic").expect("the edit succeeds")
    });
    let barrier = Barrier::new(THREADS);
    thread::scope(|scope| {
        let (port, reads, barrier) = (&port, &reads, &barrier);
        let readers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..ROUNDS {
                        assert_eq!(&read_entry(port), reads);
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().expect("no panic");
        }
    });
    assert_eq!(read_editor(port.editor()), read_editor(&expected));
}

#[test]
fn threads_edit_editors_of_their_own_at_once() {
    let expected = edited(&[edit, edit_again]);
    let barrier = Barrier::new(THREADS);
    thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let mut editor = open();
                    edit(&mut editor).and_then(|()| edit_again(&mut editor))?;
                    Ok::<_, EditError>(read_editor(&editor))
                })
            })
            .collect();
        for worker in workers {
            let view = worker.join().expect("no panic").expect("the edits succeed");
            assert_eq!(view, expected);
        }
    });
}
