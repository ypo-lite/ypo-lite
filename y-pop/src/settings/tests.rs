use y_core::xml::{Document, Element, Node};

use super::*;
use crate::config;
use crate::model::ConfigTree;
use crate::variables::Variable;

fn builtins() -> Vec<Variable> {
    vec![Variable {
        name: "win".into(),
        value: r"C:\Windows".into(),
    }]
}

fn load(text: &str) -> Settings {
    Settings::load_str(text, &mut builtins()).expect("valid XML")
}

const SHIPPED: &str = include_str!("../../rsc/y-pop.xml");

/// A node per line: everything the running reader made of the file.
fn dump(tree: &ConfigTree) -> Vec<String> {
    tree.all_nodes()
        .map(|id| format!("{:?}", tree.node(id)))
        .collect()
}

/// Every comment in a file, wherever it is, sorted.
fn comment_texts(text: &str) -> Vec<String> {
    fn walk(element: Element<'_>, out: &mut Vec<String>) {
        for node in element.children_with_comments() {
            match node {
                Node::Comment(comment) => out.push(comment.text().to_owned()),
                Node::Element(child) => walk(child, out),
            }
        }
    }
    let document = Document::parse(text).expect("valid XML");
    let mut out: Vec<String> = document
        .prolog_comments()
        .chain(document.epilog_comments())
        .map(|comment| comment.text().to_owned())
        .collect();
    walk(document.root(), &mut out);
    out.sort();
    out
}

fn only_group(settings: &Settings) -> &Item {
    match &settings.roots[0] {
        Entry::Item(item) => item,
        Entry::Comment(_) => panic!("expected an item first"),
    }
}

fn children(item: &Item) -> Vec<&Item> {
    item.children
        .iter()
        .filter_map(|entry| match entry {
            Entry::Item(item) => Some(item),
            Entry::Comment(_) => None,
        })
        .collect()
}

#[test]
fn the_shipped_file_survives_a_round_trip() {
    let before = load(SHIPPED);
    let written = before.to_xml();

    let original = config::load_str(SHIPPED, &mut builtins()).expect("valid");
    let again = config::load_str(&written, &mut builtins()).expect("valid");
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    assert_eq!(dump(&original), dump(&again), "the same menus");

    assert_eq!(
        comment_texts(SHIPPED),
        comment_texts(&written),
        "the same comments"
    );

    let after = load(&written);
    assert_eq!(before.globals, after.globals);
    assert_eq!(before.skin, after.skin);
    assert_eq!(before.variables, after.variables);
    assert_eq!(before.prolog, after.prolog);
    assert_eq!(before.intro, after.intro);
    assert_eq!(before.roots, after.roots, "the same tree, comments and all");
}

#[test]
fn saving_twice_gives_the_same_bytes() {
    let once = load(SHIPPED).to_xml();
    let twice = load(&once).to_xml();
    assert_eq!(once, twice);
}

#[test]
fn the_shipped_file_loads_without_warnings() {
    let settings = load(SHIPPED);
    assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);
    // Only the tray menu, with the Settings entry added to it.
    assert_eq!(settings.roots.len(), 1);
    assert!(
        children(only_group(&settings))
            .iter()
            .any(|item| item.kind == ItemKind::Settings)
    );
}

#[test]
fn values_stay_raw() {
    let settings = load(SHIPPED);
    let notepad = settings
        .items()
        .find(|item| item.name.as_deref() == Some("Notepad"))
        .expect("in the default file");
    let ItemKind::App(app) = &notepad.kind else {
        panic!("an app")
    };
    assert_eq!(app.exe.as_deref(), Some(r"%win%\notepad.exe"));

    assert_eq!(
        settings.variables,
        [VarEntry::Var(Var {
            name: "docs".into(),
            value: "%mydocs%".into(),
            blank_before: false,
            extra_attrs: Vec::new(),
        })]
    );
}

#[test]
fn a_comment_directly_above_an_item_is_attached_to_it() {
    let settings = load(
        r#"<y-pop>
            <group name="G">
                <!-- about a -->
                <app name="A" exe="a.exe"/>

                <!-- about nothing -->

                <!-- about b, first line -->
                <!-- about b, second line -->
                <app name="B" exe="b.exe"/>
                <!-- trailing -->
            </group>
        </y-pop>"#,
    );

    let group = only_group(&settings);
    let [
        Entry::Item(a),
        Entry::Comment(free),
        Entry::Item(b),
        Entry::Comment(trailing),
    ] = &group.children[..]
    else {
        panic!("unexpected shape: {:?}", group.children)
    };

    assert_eq!(a.lead, [Comment::new("about a")]);
    assert_eq!(free.text, "about nothing");
    assert!(free.blank_before);
    assert_eq!(
        b.lead,
        [
            Comment::new("about b, first line"),
            Comment::new("about b, second line")
        ]
    );
    // The empty line sits above the whole group of comments.
    assert!(b.blank_before);
    assert_eq!(trailing.text, "trailing");
}

#[test]
fn an_item_after_an_empty_line_takes_no_comment_with_it() {
    let settings = load(
        r#"<y-pop><group name="G">
            <!-- loose -->

            <app name="A" exe="a.exe"/>
        </group></y-pop>"#,
    );
    let group = only_group(&settings);
    assert!(matches!(&group.children[0], Entry::Comment(c) if c.text == "loose"));
    let Entry::Item(a) = &group.children[1] else {
        panic!("an item")
    };
    assert!(a.lead.is_empty());
    assert!(a.blank_before);
}

#[test]
fn comments_around_and_before_the_first_child_are_file_notes() {
    let settings = load(
        r#"<!-- before -->
        <y-pop>
            <!-- first -->
            <!-- second -->
            <group name="G"/>
        </y-pop>
        <!-- after -->"#,
    );
    assert_eq!(settings.prolog, [Comment::new("before")]);
    assert_eq!(settings.epilog.len(), 1);

    // Directly above the first group, so the group keeps them: they move
    // with it when it is moved.
    assert!(settings.intro.is_empty());
    assert_eq!(only_group(&settings).lead.len(), 2);
}

#[test]
fn a_comment_above_variables_is_an_intro_note() {
    let settings = load(
        r#"<y-pop>
            <!-- about the variables -->
            <variables><var name="a" value="b"/></variables>
            <group name="G"/>
        </y-pop>"#,
    );
    assert_eq!(settings.intro, [Comment::new("about the variables")]);
}

#[test]
fn comments_inside_variables_and_skin_are_kept() {
    let text = r#"<y-pop>
        <variables>
            <!-- one -->
            <var name="a" value="1"/>

            <var name="b" value="%a%2"/>
        </variables>
        <skin back-color="101010">
            <!-- a skin note -->
        </skin>
        <quit/>
    </y-pop>"#;
    let settings = load(text);

    assert!(matches!(&settings.variables[0], VarEntry::Comment(c) if c.text == "one"));
    assert!(matches!(&settings.variables[2], VarEntry::Var(v) if v.blank_before));
    assert_eq!(settings.skin.notes, [Comment::new("a skin note")]);

    assert_eq!(comment_texts(text), comment_texts(&settings.to_xml()));
}

#[test]
fn the_empty_line_between_items_is_kept() {
    let text = "<y-pop>\n\t<group name=\"A\"/>\n\n\t<group name=\"B\"/>\n\t<group name=\"C\"/>\n</y-pop>\n";
    let settings = load(text);
    let blank: Vec<bool> = settings
        .roots
        .iter()
        .map(|entry| matches!(entry, Entry::Item(item) if item.blank_before))
        .collect();
    assert_eq!(blank, [false, true, false]);
    assert!(
        settings
            .to_xml()
            .contains("<group name=\"A\"/>\n\n\t<group name=\"B\"/>\n\t<group")
    );
}

#[test]
fn unknown_attributes_survive_and_are_still_warned_about() {
    let settings = load(
        r#"<y-pop flavour="mint" popup="neon">
            <group name="G" colour="red"><hotkey key="Win+G" sticky="yes"/></group>
        </y-pop>"#,
    );
    assert!(settings.warnings.iter().any(|w| w.contains("colour")));

    let written = settings.to_xml();
    for kept in [
        "flavour=\"mint\"",
        "popup=\"neon\"",
        "colour=\"red\"",
        "sticky=\"yes\"",
    ] {
        assert!(written.contains(kept), "{kept} lost in:\n{written}");
    }
}

#[test]
fn a_field_the_editor_sets_wins_over_a_kept_stale_value() {
    let mut settings = load(r#"<y-pop popup="neon"><quit/></y-pop>"#);
    settings.globals.popup = Some(Renderer::Overlay);

    let written = settings.to_xml();
    assert!(written.contains("popup=\"overlay\""), "{written}");
    assert!(!written.contains("neon"), "{written}");
}

#[test]
fn an_element_y_pop_would_reject_is_kept_as_a_comment() {
    let text = r#"<y-pop>
        <group name="G">
            <widget size="3"><part/></widget>
            <app name="No target"/>
            <app name="Fine" exe="a.exe"/>
        </group>
    </y-pop>"#;
    let settings = load(text);

    let group = only_group(&settings);
    let comments: Vec<&str> = group
        .children
        .iter()
        .filter_map(|entry| match entry {
            Entry::Comment(comment) => Some(comment.text.as_str()),
            Entry::Item(_) => None,
        })
        .collect();
    assert_eq!(comments.len(), 2, "{:?}", group.children);
    assert!(comments[0].contains("<widget size=\"3\">") && comments[0].contains("<part/>"));
    assert!(comments[1].contains("No target"));
    assert!(
        settings
            .warnings
            .iter()
            .any(|w| w.contains("kept in the file as a comment"))
    );

    // Saved, it loads like the original did — and the XML is still there.
    let written = settings.to_xml();
    let reloaded = config::load_str(&written, &mut builtins()).expect("valid");
    assert_eq!(reloaded.roots.len(), 1);
    assert!(written.contains("<widget size=\"3\">"));
    assert!(written.contains("<app name=\"No target\"/>"));
}

#[test]
fn a_leaf_element_s_children_are_kept_not_dropped() {
    let settings = load(
        r#"<y-pop><app name="A" exe="a.exe"><hotkey key="Win+A"/><app name="inner" exe="b.exe"/></app></y-pop>"#,
    );
    let Entry::Item(app) = &settings.roots[0] else {
        panic!("an item")
    };
    assert_eq!(app.hotkeys.len(), 1);
    assert_eq!(app.inner_comments.len(), 1);
    assert!(
        settings
            .to_xml()
            .contains("<app name=\"inner\" exe=\"b.exe\"/>")
    );
}

#[test]
fn an_incomplete_skin_part_is_kept_as_a_comment() {
    let settings = load(
        r#"<y-pop><skin><border size="1"/><background-gradient start="000000" end="ffffff"/></skin><quit/></y-pop>"#,
    );
    assert!(settings.skin.overrides.border.is_none());
    assert!(settings.skin.overrides.background_gradient.is_some());
    assert_eq!(settings.skin.notes.len(), 1);
    assert!(settings.to_xml().contains("<border size=\"1\"/>"));
}

#[test]
fn a_variable_without_a_value_is_kept_as_a_comment() {
    let settings = load(r#"<y-pop><variables><var name="x"/></variables><quit/></y-pop>"#);
    assert!(
        matches!(&settings.variables[0], VarEntry::Comment(c) if c.text.contains("name=\"x\""))
    );
}

#[test]
fn every_app_field_round_trips() {
    let text = r#"<y-pop shell-execute="true" show-shortcuts="false" popup="overlay" popup-style="list" theme="dark">
        <group name="G" icon="%win%\explorer.exe,3" tooltip="t" default="true" autoopen="true" show-shortcuts="true" popup="native">
            <hotkey key="Win+G"/>
            <hotkey key="Win+H" override="true"/>
            <app name="A" exe="%win%\a.exe" param="%clipboard%" path="C:\" shell="false" elevate="true" events="start,exit,sleep" browse="true" filter="*.sln" depth="3" open-folder="all"/>
            <app name="Store" app-id="Pkg!App"/>
            <separator name="Tools"/>
            <separator/>
            <break/>
            <about name=""/>
            <quit/>
            <settings/>
        </group>
    </y-pop>"#;
    let settings = load(text);
    assert!(
        settings
            .warnings
            .iter()
            .all(|warning| warning.contains("event")),
        "{:?}",
        settings.warnings
    );

    let again = load(&settings.to_xml());
    assert_eq!(settings.globals, again.globals);
    assert_eq!(settings.roots, again.roots);

    let group = only_group(&settings);
    let a = children(group)[0];
    let ItemKind::App(app) = &a.kind else {
        panic!("an app")
    };
    assert_eq!(app.shell, Some(false));
    assert!(app.on_start && app.on_exit && app.elevate && app.browse);
    assert_eq!(app.other_events, ["sleep"]);
    assert_eq!(app.depth, Some(3));
    assert_eq!(app.open_folder, Some(OpenFolder::All));
    assert_eq!(group.popup, Some(Renderer::Native));
    assert!(group.hotkeys[1].can_override);
    // An explicit empty name is not the same as no name.
    assert_eq!(children(group)[5].name.as_deref(), Some(""));
    assert_eq!(children(group)[3].name, None);
}

#[test]
fn a_skin_round_trips() {
    let text = r##"<y-pop theme="auto"><skin back-color="#101010" hover-back-color="ff880080" padding="2" margin="4" edge="hover">
        <border size="1" round="8" color="3a3a3a"/>
        <background-gradient start="2b2b2b" end="1a1a1a" direction="v"/>
        <icon-gradient start="ffd996" end="000000"/>
    </skin><quit/></y-pop>"##;
    let settings = load(text);
    assert!(settings.warnings.is_empty(), "{:?}", settings.warnings);

    let overrides = settings.skin.overrides;
    assert_eq!(
        overrides.back,
        Some(y_core::overlay::Color::rgb(0x10, 0x10, 0x10))
    );
    assert_eq!(
        overrides.hover_back,
        Some(y_core::overlay::Color::rgba(0xff, 0x88, 0, 0x80))
    );

    let written = settings.to_xml();
    let again = load(&written);
    assert_eq!(settings.skin, again.skin);

    // The real reader sees the same skin.
    let one = config::load_str(text, &mut builtins()).expect("valid");
    let two = config::load_str(&written, &mut builtins()).expect("valid");
    assert_eq!(one.defaults.theme_overrides, two.defaults.theme_overrides);
}

#[test]
fn a_file_with_no_skin_and_no_variables_writes_neither() {
    let written = load("<y-pop><quit/></y-pop>").to_xml();
    assert!(
        !written.contains("skin") && !written.contains("variables"),
        "{written}"
    );
}

#[test]
fn ids_are_unique_and_new_ones_do_not_collide() {
    let mut settings = load(SHIPPED);
    let mut seen: Vec<ItemId> = settings.items().map(|item| item.id).collect();
    let count = seen.len();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), count);

    let fresh = settings.new_item(ItemKind::Separator);
    assert!(!seen.contains(&fresh.id));
}

#[test]
fn an_edit_is_written_and_read_back() {
    let mut settings = load(SHIPPED);
    let new = settings.new_item(ItemKind::Quit);
    let Entry::Item(group) = &mut settings.roots[0] else {
        panic!("an item")
    };
    group.children.push(Entry::Item(Item {
        name: Some("Bye".into()),
        ..new
    }));
    settings.globals.theme = Some(Theme::Dark);

    let written = settings.to_xml();
    let tree = config::load_str(&written, &mut builtins()).expect("valid");
    assert!(tree.warnings.is_empty(), "{:?}", tree.warnings);
    assert!(dump(&tree).iter().any(|node| node.contains("Bye")));
    assert_eq!(tree.defaults.theme, Theme::Dark);
    // Nothing else was lost on the way.
    assert_eq!(comment_texts(SHIPPED), comment_texts(&written));
}

#[test]
fn saving_replaces_the_file_and_keeps_one_backup_per_session() {
    let directory = std::env::temp_dir().join(format!("y-pop-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    let path = directory.join("y-pop.xml");

    // No file yet: nothing to back up.
    let mut settings = load("<y-pop><quit/></y-pop>");
    settings.save_to(&path).expect("writes");
    assert!(!directory.join("y-pop.xml.bak").exists());
    assert!(!directory.join("y-pop.xml.tmp").exists());

    // A file by hand, then the first save of a new session.
    std::fs::write(&path, "<y-pop><about/></y-pop>").expect("writes");
    let mut session = load("<y-pop><quit/></y-pop>");
    session.save_to(&path).expect("writes");
    assert_eq!(
        std::fs::read_to_string(directory.join("y-pop.xml.bak")).expect("backup"),
        "<y-pop><about/></y-pop>"
    );

    // The second save leaves that backup alone.
    session.globals.show_shortcuts = Some(false);
    session.save_to(&path).expect("writes");
    assert_eq!(
        std::fs::read_to_string(directory.join("y-pop.xml.bak")).expect("backup"),
        "<y-pop><about/></y-pop>"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("file"),
        session.to_xml()
    );

    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn malformed_xml_is_an_error_with_a_line() {
    let error = Settings::load_str("<y-pop>\n<group></y-pop>", &mut builtins()).expect_err("bad");
    assert_eq!(error.line, 2);
}

#[test]
fn the_file_notes_read_as_one_text_in_file_order() {
    let settings = load(
        "<!-- first -->\n<y-pop>\n<!-- second -->\n<variables><var name=\"a\" value=\"b\"/></variables>\n<quit/>\n</y-pop>\n<!-- last\n\nstill last -->",
    );
    assert_eq!(settings.file_notes_count(), 3);
    assert_eq!(
        settings.file_notes_text(),
        "first\n\nsecond\n\nlast\n\nstill last"
    );
}

#[test]
fn editing_the_file_notes_merges_them_and_blank_removes_them() {
    let mut settings = load("<!-- a --><y-pop><!-- b --><variables/><quit/></y-pop><!-- c -->");

    // Reporting what is already there is not an edit.
    let same = settings.file_notes_text();
    assert!(!settings.set_file_notes_text(&same));
    assert_eq!(
        (
            settings.prolog.len(),
            settings.intro.len(),
            settings.epilog.len()
        ),
        (1, 1, 1)
    );

    assert!(settings.set_file_notes_text("one note\nfor everything"));
    assert_eq!(settings.file_notes_count(), 1);
    assert_eq!(settings.file_notes_text(), "one note\nfor everything");
    assert!(settings.to_xml().starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!--\n\tone note\n\tfor everything\n-->\n<y-pop"));

    assert!(settings.set_file_notes_text("  \n "));
    assert_eq!(settings.file_notes_count(), 0);
    assert!(!settings.to_xml().contains("<!--"));
}

#[test]
fn a_text_box_s_line_breaks_are_not_an_edit() {
    let mut settings = load("<!-- one\n\ntwo -->\n<y-pop><quit/></y-pop>");
    // What a WinUI text box reports for the same text.
    assert!(!settings.set_file_notes_text("one\r\rtwo"));
    assert!(!settings.set_file_notes_text("one\r\n\r\ntwo"));
    assert!(settings.set_file_notes_text("one\r\rtwo!"));
    assert_eq!(settings.file_notes_text(), "one\n\ntwo!");
}

#[test]
fn an_item_previews_as_the_xml_a_save_would_write() {
    let settings = load(SHIPPED);
    let notepad = settings
        .items()
        .find(|item| item.name.as_deref() == Some("Notepad"))
        .expect("in the default file");
    let xml = notepad.to_xml();
    assert!(xml.starts_with("<!--\n\toverride=\"true\""), "{xml}");
    assert!(
        xml.contains("<app name=\"Notepad\" exe=\"%win%\\notepad.exe\">"),
        "{xml}"
    );
    assert!(
        xml.contains("\t<hotkey key=\"Win+N\" override=\"true\"/>"),
        "{xml}"
    );
    assert!(xml.ends_with("</app>"), "{xml}");

    // And it is part of what the whole file says.
    let whole = settings.to_xml();
    for line in xml.lines().skip(1) {
        assert!(whole.contains(line.trim_start()), "{line}");
    }
}

#[test]
fn expand_text_uses_the_files_variables_builtins_and_leaves_the_clipboard() {
    let settings = load(
        r#"<y-pop><variables><var name="tools" value="%win%\tools"/><var name="git" value="%tools%\git"/></variables></y-pop>"#,
    );
    let mut lookup = builtins();
    assert_eq!(
        settings.expand_text(r"%git%\git.exe %clipboard% %nope%", &mut lookup),
        r"C:\Windows\tools\git\git.exe %clipboard% %nope%"
    );
}

#[test]
fn launch_app_expands_and_settles_the_shell_choice() {
    let settings = load(
        r#"<y-pop shell-execute="true"><app name="A" exe="%win%\notepad.exe" param="-x %clipboard%" path="%win%" events="start"/>
           <app name="B" exe="b.exe" shell="false" elevate="true"/></y-pop>"#,
    );
    let apps: Vec<_> = settings
        .items()
        .filter_map(|item| match &item.kind {
            ItemKind::App(fields) => Some(fields.as_ref()),
            _ => None,
        })
        .collect();
    let a = settings.launch_app(apps[0], &mut builtins());
    assert_eq!(a.exe.as_deref(), Some(r"C:\Windows\notepad.exe"));
    assert_eq!(a.param.as_deref(), Some("-x %clipboard%"));
    assert_eq!(a.path.as_deref(), Some(r"C:\Windows"));
    assert!(a.shell && !a.elevate && a.events.contains(crate::model::EventMask::START));
    let b = settings.launch_app(apps[1], &mut builtins());
    assert!(!b.shell && b.elevate);
}

const WITH_SEARCH: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<y-pop>
	<variables>
		<var name="tools" value="%win%\tools"/>
	</variables>
	<!-- the launcher -->
	<search hotkey="Alt+Space" override="true" collapsed="true" results="flat" row-actions="false" history="false" future="x">
		<recent count="7"/>

		<!-- natural order -->
		<menus browse="a-z" include="y-pop;Admin tools" later="1"/>
		<start-menu exclude="*uninstall*"/>
		<!-- only here -->
		<group name="Admin tools" keywords="admin root">
			<app name="Registry" exe="%tools%\regedit.exe"/>
			<app name="Services" exe="services.msc" keywords="svc"/>
		</group>
		<app name="Scripts" exe="%tools%\Scripts" browse="true" depth="2"/>
		<plugin name="Calculator" src="plugins\calc.wasm" prefix="=" allow="clipboard"/>
		<!-- trailing -->
	</search>
	<group name="Work">
		<app name="Notepad" exe="notepad.exe" keywords="text edit"/>
	</group>
</y-pop>
"#;

fn search_of(settings: &Settings) -> &SearchSettings {
    settings.search.as_ref().expect("a <search>")
}

#[test]
fn search_is_loaded_as_elements_not_as_a_comment() {
    let settings = load(WITH_SEARCH);
    assert!(
        !settings
            .warnings
            .iter()
            .any(|warning| warning.contains("kept in the file as a comment")),
        "{:?}",
        settings.warnings
    );

    let search = search_of(&settings);
    assert_eq!(search.hotkey.as_deref(), Some("Alt+Space"));
    assert!(search.can_override);
    assert_eq!(search.collapsed, Some(true));
    assert_eq!(search.results, Some(ResultsMode::Flat));
    assert_eq!(search.row_actions, Some(false));
    assert_eq!(search.history, Some(false));
    assert_eq!(search.lead, [Comment::new("the launcher")]);
    assert_eq!(
        search.extra_attrs,
        [("future".to_owned(), "x".to_owned())],
        "unknown attributes are kept"
    );

    let kinds: Vec<&str> = search
        .children
        .iter()
        .map(|child| match child {
            SearchEntry::Comment(_) => "comment",
            SearchEntry::Item(item) => item.kind.element(),
            SearchEntry::Section(section) => section.kind.element(),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "recent",
            "menus",
            "start-menu",
            "group",
            "app",
            "plugin",
            "comment"
        ],
        "comments directly above an element are attached to it"
    );
    let SearchEntry::Section(menus) = &search.children[1] else {
        panic!("menus")
    };
    assert_eq!(menus.lead, [Comment::new("natural order")]);
    assert!(menus.blank_before, "the empty line above it is kept");
    // Only the tray menu is a root.
    assert_eq!(settings.roots.len(), 1);
}

#[test]
fn search_survives_a_round_trip_with_every_kind_of_child() {
    let before = load(WITH_SEARCH);
    let written = before.to_xml();

    let original = config::load_str(WITH_SEARCH, &mut builtins()).expect("valid");
    let again = config::load_str(&written, &mut builtins()).expect("valid");
    assert_eq!(original.warnings, again.warnings);
    assert_eq!(dump(&original), dump(&again), "the same nodes");
    assert_eq!(
        format!("{:?}", original.search),
        format!("{:?}", again.search),
        "the same search"
    );

    assert_eq!(
        comment_texts(WITH_SEARCH),
        comment_texts(&written),
        "the same comments"
    );
    let after = load(&written);
    assert_eq!(search_of(&before), search_of(&after));
    assert_eq!(before.roots, after.roots);
}

#[test]
fn a_file_with_search_is_written_byte_for_byte_again() {
    assert_eq!(load(WITH_SEARCH).to_xml(), WITH_SEARCH);
}

#[test]
fn search_items_keep_a_hotkey_the_reader_ignores() {
    let text =
        r#"<y-pop><search><group name="G"><hotkey key="Win+T"/><quit/></group></search></y-pop>"#;
    let settings = load(text);
    assert!(
        settings
            .warnings
            .iter()
            .any(|warning| warning.contains("ignored")),
        "the reader's own warning is reported: {:?}",
        settings.warnings
    );
    assert!(settings.to_xml().contains("<hotkey key=\"Win+T\"/>"));
}

#[test]
fn a_second_search_and_a_search_in_a_group_are_kept_as_comments() {
    let settings = load(
        r#"<y-pop><search hotkey="Alt+Space"/><search hotkey="Alt+Q"/><group name="G"><search/><quit/></group></y-pop>"#,
    );
    assert_eq!(search_of(&settings).hotkey.as_deref(), Some("Alt+Space"));
    let written = settings.to_xml();
    assert_eq!(
        comment_texts(&written),
        ["<search hotkey=\"Alt+Q\"/>", "<search/>"]
    );
}

#[test]
fn a_bad_search_value_stays_as_it_was() {
    let text = r#"<y-pop><search results="grid"><menus browse="tree"/><recent count="many"/></search></y-pop>"#;
    let written = load(text).to_xml();
    assert!(written.contains("results=\"grid\""), "{written}");
    assert!(written.contains("browse=\"tree\""), "{written}");
    assert!(written.contains("count=\"many\""), "{written}");
}

#[test]
fn renaming_a_variable_reaches_the_search_items() {
    let mut settings = load(WITH_SEARCH);
    assert!(
        settings
            .variable_users(0)
            .iter()
            .any(|user| matches!(user, UsedIn::Item { .. }))
    );
    let users = settings.variable_users(0).len();
    assert_eq!(users, 2, "Registry and Scripts");

    let rewritten = settings.rename_variable(0, "utils").expect("free name");
    assert_eq!(rewritten, 2);
    let written = settings.to_xml();
    assert!(written.contains(r"%utils%\regedit.exe"), "{written}");
    assert!(written.contains(r"%utils%\Scripts"), "{written}");
}

const WITH_ACTIONS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<y-pop>
	<search hotkey="Alt+Space">
		<recent count="5"/>
		<actions menu-key="Ctrl+K" quick-pick="false" extra="kept">
			<!-- what Enter does -->
			<action do="open" key="Enter"/>
			<action do="edit" key="Ctrl+E" button="true"/>
			<action do="copy-path"/>
			<!-- not an action yet -->
			<action do="fly" key="Ctrl+Nonsense" button="maybe" future="1"/>
			<action key="Ctrl+J"/>
		</actions>
	</search>
	<group name="Work">
		<app name="Notepad" exe="notepad.exe"/>
	</group>
</y-pop>
"#;

fn actions_of(settings: &Settings) -> (&Option<String>, &Option<bool>, &Vec<ActionEntry>) {
    let search = settings.search.as_ref().expect("a <search>");
    for child in &search.children {
        if let SearchEntry::Section(SearchSection {
            kind:
                SectionKind::Actions {
                    menu_key,
                    quick_pick,
                    entries,
                },
            ..
        }) = child
        {
            return (menu_key, quick_pick, entries);
        }
    }
    panic!("no <actions> in {:?}", search.children)
}

#[test]
fn actions_are_loaded_as_elements_with_their_comments() {
    let settings = load(WITH_ACTIONS);
    assert!(
        !settings
            .warnings
            .iter()
            .any(|warning| warning.contains("kept in the file as a comment")),
        "{:?}",
        settings.warnings
    );

    let (menu_key, quick_pick, entries) = actions_of(&settings);
    assert_eq!(menu_key.as_deref(), Some("Ctrl+K"));
    assert_eq!(*quick_pick, Some(false));
    assert_eq!(entries.len(), 7);
    assert!(matches!(&entries[0], ActionEntry::Comment(c) if c.text == "what Enter does"));
    let ActionEntry::Action(edit) = &entries[2] else {
        panic!("an action")
    };
    assert_eq!(
        (edit.id.as_deref(), edit.key.as_deref(), edit.button),
        (Some("edit"), Some("Ctrl+E"), Some(true))
    );
    let ActionEntry::Action(odd) = &entries[5] else {
        panic!("an action")
    };
    assert_eq!(odd.key.as_deref(), Some("Ctrl+Nonsense"), "kept as written");
    assert_eq!(odd.button, None, "an unreadable value is not taken…");
    assert!(
        odd.extra_attrs.iter().any(|(name, _)| name == "button"),
        "…but kept"
    );
    let ActionEntry::Action(no_id) = &entries[6] else {
        panic!("an action")
    };
    assert_eq!(no_id.id, None);
}

#[test]
fn actions_are_written_back_byte_for_byte() {
    assert_eq!(load(WITH_ACTIONS).to_xml(), WITH_ACTIONS);
}

#[test]
fn actions_survive_a_round_trip_through_the_real_reader() {
    let written = load(WITH_ACTIONS).to_xml();
    let original = config::load_str(WITH_ACTIONS, &mut builtins()).expect("valid");
    let again = config::load_str(&written, &mut builtins()).expect("valid");
    assert_eq!(original.warnings, again.warnings);
    assert_eq!(
        original.search.unwrap().actions,
        again.search.unwrap().actions
    );
}

#[test]
fn an_actions_element_with_something_else_inside_is_kept_as_a_comment() {
    let settings = load(r#"<y-pop><search><actions><widget/></actions></search></y-pop>"#);
    let search = settings.search.as_ref().unwrap();
    assert!(
        matches!(&search.children[0], SearchEntry::Comment(c) if c.text.contains("<widget/>")),
        "{:?}",
        search.children
    );
}
