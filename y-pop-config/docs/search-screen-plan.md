# The Search screen — plan for `y-pop` search task 11.3

**11.3.6–11.3.8 are built** (from [the review of 2026-10-30](#review-2026-10-30); unit-tested, compile-checked, not yet run live). What is left: the live pass, 11.3.9, and 5.2. Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live.
Update a row in the same change that does the work, as in [`plan.md`](plan.md).

Goal (`y-pop/docs/search-plan.md`, 11.3): **search configured entirely from
`y-pop-config`** — everything in `<search>` that `y-pop/docs/user-manual.md`
("Configuring the search") lets a person write, without touching the XML.

Read first: `CLAUDE.md`, `plan.md` ("Working notes"), `ui-kit/docs/ui-toolkit.md`.

## What is already there

- **The model is done** (`y_pop::settings`, search phase 1.7): `Settings::search:
  Option<SearchSettings>` — `hotkey`, `can_override`, `collapsed`, `results`,
  `row_actions`, `history`, `extra_attrs`, and `children: Vec<SearchEntry>`
  (`Comment`, `Item` for a search-only group or browsed folder, `Section`
  with `Recent`, `Menus`, `StartMenu`, `Plugin`, `Actions`). Loaded, saved and
  round-trip tested; a screen-less save keeps all of it.
- **Items in `search.children` are ordinary `Item`s** with ids from the same
  counter as the tray menus' — an `ItemId` is unique across both.
- **The outline, inspector, hotkey picker, icon picker and app form** already
  edit an `Item` by `ItemId` (`menu_view`, `inspector`, `app_form`,
  `menu_edit`). What they cannot do is *find* a search-only item: every finder in
  `tree.rs` walks `settings.roots` only.
- 11.1/11.2/11.4 exist: `--select`, `--new-app`, `keywords`, `index`.

## Decisions

Confirmed by the user on 2026-10-30: keys are a draft until OK, for item hotkeys
too (6.3); the page of 11.3.8; only the Browse list reorders (8.3). Below, the
first draft's, kept for the record: 1 and 2 are superseded by 11.3.8.

1. **Navigation.** One **Search** entry in the pane, after the tray menus, with
   the search-only groups/folders as children of it (like the tray menus are
   screens of their own). The Search page itself is: Basics, Sections, Actions.
   *Alternative:* three nav entries (Search, Sections, Actions) — more clicks, less
   scrolling.
2. **"No sections written" is shown, not hidden.** The file's rule — one
   section written turns the defaults off — is the trap in the manual's "Watch
   out". The Sections card shows the three defaults greyed with "Using the
   defaults", and a **Customize** button that writes them explicitly
   (`<recent/><menus/><start-menu/>`) before any edit. Removing the last
   section does not silently return to the defaults: it says so.
3. **Plugins stay read-only.** A `<plugin>` row is listed ("kept as written"),
   movable and removable, never edited (the model keeps all its attributes
   in `extra_attrs`; there is no plugin runtime yet).
4. **`<start-menu exclude>`** is editable even though y-pop ignores it until
   search phase 9; the card says "not used yet".
5. **Renaming a tray menu updates `<menus include>`** (like variable rename
   updates its users). A name containing `;` can't be written in `include`:
   say so on the rename instead of writing a broken list.
6. **Edit on a search-only item** needs a path: extend `ipc::path_of` /
   `roots::resolve_path` with `s/<n>` — the n-th *item* among `<search>`'s
   children, then positions as before (step 11.3.5).

## Review (2026-10-30)

Read against y-pop's reader and its search state. Every finding marked *probe*
was run against `y_pop::config` in a throwaway test (since removed); the others
come from reading the named code. The 181 existing tests pass: they pin the
logic, so none of this shows up in them.

### Why the plan let this through

- **Nobody owned the key picker.** "What is already there" says the picker edits
  an `Item`; "Order and size" says 11.3.3 reuses it; no task adapts it. So 3.2 and
  3.5 shipped text boxes, and 3.2 was marked 👁, "done", with "not done" in its
  own notes.
- **The page mirrors the XML, not what y-pop does.** Children of `<search>` became
  rows of one table. y-pop treats them as three different things (D2), so the
  screen offers controls that do nothing.
- **The screen has no design.** `spec.md` does not mention it, the mock-up neither;
  every other screen has one. Nothing but the XML schema shaped it.
- **"Decisions to confirm" were never confirmed.** They were built, and 1 and 2
  were then built differently from what they say.
- **"Done when" was never checked** (all live), and the tests prove reading and
  writing, not that a control means what it says.

### Design

| # | Finding | Fix |
|---|---|---|
| D1 | **Every key is a raw text box**: the search hotkey, nine action shortcuts, the menu key. Each keystroke writes the model, so a half-typed `Ctr` is in the file if saved, and the first one writes the whole `<actions>` list. The toolkit forbids this on purpose: "never typed", so "there is no invalid state to design around". | 11.3.6 |
| D2 | **The sections table is one list of three things.** Recent is always first, wherever it is written (`state.rs`, the empty box). Only Tray menus and groups are rows under BROWSE (`sections.rs`). Start menu, folders and plugins are never listed. So ↑↓ on Recent, Start menu, a folder or a plugin changes nothing a person can see, and the card's title, "What the empty box lists", is untrue for them. | 11.3.8 |
| D3 | **Settings that depend on each other are shown apart.** `history="false"` empties Recent (`search_window.rs:376`) while its row still says "The last 5 things you opened". `row-actions="false"` leaves the Button column live. | 11.3.8 |
| D4 | **The hotkey conflicts nothing checks.** `shared_with` walks items only, so a search hotkey equal to a menu's is silent on both pages, and y-pop refuses one of them at start-up (*probe*: no load warning). Plan 3.2 knew; no task followed. `keycheck::reserved_for` already knows `Win+S` and the like, and is unused here. | 11.3.6 |
| D5 | **The file's vocabulary leaks.** "y-pop.xml has no `<search>`", "No `<actions>` is written", "(default)", "(added by y-pop)". With every edit writing out what it needs, "written or not" is no longer the user's concern. Only a way back matters: actions have one, **sections have none** short of turning search off. | 11.3.8 |
| D6 | **"Turn search off" is a small button that deletes the most.** It removes every search-only group and action with one level of Undo, gone after a save. Deleting one tray menu asks first. | 11.3.8 |
| D7 | **A section's actions take two clicks and show no sign of opening.** A name is a plain button; the group's page is behind it, then *Edit it*. `Open` is protected from Remove "or Enter would do nothing", yet clearing its shortcut does exactly that (*probe*). | 11.3.8, B8 |
| D8 | Row keys are positions (`s{index}`, `a{index}`): rule 7 of the toolkit, and the fields of a moved row are reused for another. | 11.3.8 |

### Bugs

| # | Severity | What happens | Evidence |
|---|---|---|---|
| B1 | **High** | Deleting a tray menu leaves its name in `include`. If it was the only one, y-pop drops **every** tray menu from search; the only trace is a start-up warning. The rename path handles this; the delete path was forgotten. | *probe*: `includes_menu("Games")` false, counted `[]` |
| B2 | **High** | Backspacing a tray menu's name to nothing, then typing a new one, loses it from `include`, with the same result as B1. The plan calls this acceptable (2.7); selecting and retyping a name is normal use. | `rename_in_include` on `""`, *probe* |
| B3 | **High** | With one tray menu named with a `;` and no `include`, **no** menu can be unticked: `set_include` refuses the whole list and `.unwrap_or(false)` hides it. The clicks do nothing. | *probe*: `changed=false` |
| B4 | Medium | Decision 5 says a rename to a name with `;` "says so"; nothing does, and the menu leaves `include` silently. | `menu_edit.rs:793`, *probe* |
| B5 | Medium | **Search-only items are missing from `Settings::items()`**, which stays tray-only (*probe*: 0). So they get no icons (`main.rs:704`), "already in this menu" never flags (`menu_view.rs:699`), the status bar undercounts (`main.rs:1248`), and a variable's usage shows a blank name (`variables.rs:269`). Item 1.1 fixed `tree.rs` only. | *probe* |
| B6 | Medium | `Recent` at 0 says "Nothing (switched off)". y-pop still shows the Recent heading with an *All recent* row. Off is having no `<recent>`. | `state.rs:1243–1252` |
| B7 | Medium | The screen disagrees with y-pop, against the plan's central promise. Menu key `Enter`: it shows `Enter` on an implied `open`, y-pop gives it no key. An `<action>` y-pop drops (unknown or a duplicate) still takes part in `key_problems`, blaming the wrong row. An `<app>` without `browse` shows as "a browsed folder"; y-pop drops it. A `<plugin>` without `src` likewise. | *probe* |
| B8 | Medium | Clearing `open`'s shortcut leaves Enter doing nothing on a result, with no warning. Moving Enter to another action is legitimate, so warn when nothing holds Enter rather than refuse. | *probe* |
| B9 | Low | Clearing the hotkey leaves `<search override="true"/>`. | *probe* |
| B10 | Low | `count="200"` is valid for y-pop but over the box's 100: opening the row coerces it, and the file is rewritten to 100 unasked (the toolkit's `NumberBox` trap). | *probe*, for the apply step |
| B11 | Low | A second `<recent>` is promoted by Remove: the row comes back with the ignored one's settings. | *probe* |
| B12 | Low | `move_section` at the end of the list writes the defaults out and reports "no change". The UI's disabled arrows hide it; fix it while here. | *probe* |
| B13 | Low | Stale text: `search_view.rs:13–15` (says no page exists), `ui-checklist.md` rows 2, 5, 7, and `remove_section`'s doc, which says the last removal returns to the defaults: it can't, `<menus/>` stays (*probe*). | reading |

Checked and fine: a problem caption under an action row does not remount the row
(the table keys it), and "search off" is true (y-pop offers *Open Search* only
with a `<search>`).

## Phases

Each ends with `cargo clippy --workspace --all-targets` clean and
`cargo test --workspace` green, and can be merged alone.

### 11.3.1 — Finders that see both lists (logic, no UI)

| | Task | Status |
|---|---|---|
| 1.1 | `tree::item`, `item_mut`, `locate`, `root_of`, `groups` also see the `Item`s of `search.children`. **As built:** a private `Top` (`Roots`/`Search`) and `find_path`; `locate` returns `None` for a *top-level* search-only item (its list holds sections and comments of another type), so `move_item`, `duplicate`, `delete` and `wrap_in_group` refuse it harmlessly; nested items work unchanged. `groups` lists the search-only ones as "Only in search › …". New `tree::is_search_only`. | ✅ |
| 1.2 | Top-level search-only items are added, removed and found by `search_only` (`roots`, `find`, `position`, `at_position`, `add` for Group or Folder only, `delete`); the defaults are written out before the first is added. **Moving** one among the others is a row move of the sections list (`search_edit::move_section`), because sections and items share one order — not a `tree` move. | ✅ |
| 1.3 | (folded into 1.2) | ✅ |
| 1.4 | Tests (`search_only/tests.rs`): both lists found by id at any depth, edits inside a search-only group like a tray menu, Move-to across both, add/delete leaving sections and comments alone, unique ids. | ✅ |

**Done when:** a test moves, duplicates and deletes search-only items without
touching sections, and `menu_tests` still passes unchanged.

### 11.3.2 — Search settings as plain functions (logic, no UI)

A module `search_edit.rs`, no UI, all tested (the `roots.rs` model).

| | Task | Status |
|---|---|---|
| 2.1 | `ensure`, `turn_off`; `<search>` is created by the first edit that changes something and removed **only** by `turn_off` — **changed from the draft**: an empty `<search/>` is not nothing, it turns search on, so it is never dropped for being empty. | ✅ |
| 2.2 | Basics: `set_hotkey`, `set_override`, `set_collapsed`, `set_monitor`, `set_results`, `set_row_actions`, `set_history`; a no-op never creates `<search>`. | ✅ |
| 2.3 | Sections: `sections()` (what y-pop will use; `Row.written`, `Sections.defaults`, pinned to y-pop's own reading by a test), `customize`, `addable`/`add_section`, `can_remove` (not `<menus>`), `remove_section`, `move_section`, addressed by `SectionKey`. Every edit writes the implied entries out first. | ✅ |
| 2.4 | `recent_count`/`set_recent_count`, `menus_browse`/`set_menus_browse`, `include_names`/`set_include` (a set of menu names; all or none writes no `include`; `;` refused), `unknown_includes`, `start_exclude`/`set_start_exclude`. | ✅ |
| 2.5 | Actions: `actions()` (defaults when none written, `open` implied first), `customize_actions`, `add_action`/`addable_actions`, `remove_action` (not `open`), `move_action`, `set_action`, `set_action_key`, `set_action_button`, `set_menu_key`, `set_quick_pick`, `reset_actions`. | ✅ |
| 2.6 | `key_problems` uses y-pop's own `Chord::parse` and its "already another action's" rule; a test checks it finds exactly the problems y-pop warns about. | ✅ |
| 2.7 | `rename_in_include`, wired into the tray menu name edit (`menu_edit`); tested through `edit_menu`. A name emptied while typing breaks the link (the next keystroke has nothing to match). | ✅ |

**Done when:** every edit has a test that writes the settings and reads them
back with the real reader, with no load warnings.

### 11.3.3 — The Search page

| | Task | Status |
|---|---|---|
| 3.1 | `Screen::Search`, nav entry **Search** in the Settings group (decision 1 in short: with General, Skin and Variables; the search-only pages will be its children), tag `"search"`, remembered in `prefs` (`LastScreen::Search`), `Msg::Search(SearchMsg)`. | 👁 |
| 3.2 | **Basics:** hotkey, Override, Results, Collapsed, Row buttons, History as `setting` cards. **Not done:** the hotkey is *typed* (validated with `Hotkey::parse`), not picked with the shared key picker, and there is no "Also used by X" check against item hotkeys. **Replaced by 11.3.6.** | 👁 |
| 3.3 | Off state: a note and **Turn search on**; **Turn search off** at the end of the basics (Undo). | 👁 |
| 3.4 | **Sections:** a `Table` (name button, summary, ↑ ↓ Remove) with the selected row's fields as its detail panel; implied rows say "(default)" under the decision-2 note; add buttons for what is missing. Search-only items are rows (move, remove). **Replaced by 11.3.8.** | 👁 |
| 3.5 | **Actions:** a `Table` (action name, typed shortcut, Button switch, ↑ ↓ Remove), add buttons, menu key, quick-pick, reset. **Changed from the draft:** an action's identity is not a combo (a refused change would leave the combo showing a value the model does not hold); to change one, remove it and add the other. **Shortcuts replaced by 11.3.6.** | 👁 |
| 3.6 | Undo for TurnOff, Remove (section), RemoveAction and ResetActions (`SearchMsg::undo_label`); dirty flag and Save as for every screen. | ✅ |
| 3.7 | `ui-checklist.md` has a "Search" table (8 rows, all 👁). | ✅ |

**Done when:** 👁 on the user's machine: turn search on, set a hotkey, reorder
sections, restrict `include`, add an action, Save; y-pop reloads and the search
behaves; the file diffs by the lines changed only.

### 11.3.4 — Search-only groups and folders

| | Task | Status |
|---|---|---|
| 4.1 | A search-only group or folder opens the menu screen with that item as its root (`Screen::SearchOnly`), listed under a nav group **Only in search** (shown when there are any) — **changed from the draft:** a group of its own, like *Tray menus*, not children of Search (a selectable parent item is untested in this `NavigationView`). Remembered across runs (`only:N`). | 👁 |
| 4.2 | *Add group* and *Add folder* in the sections card (they write the default sections out first) and open the new page. A row's *Edit it* opens an existing one. | 👁 |
| 4.3 | Hidden (not disabled) for anything in a search-only tree: Default item, hotkeys, "open on hover", menu style, shortcuts, "Turn into a menu", and the move-earlier/later arrows of the root. **Checked:** y-pop ignores a `<hotkey>` in a search-only group with a warning (`config.rs` test), so nothing is lost. | 👁 |
| 4.4 | Nothing to refuse: the menu screen never adds at the top level; only the Search page adds, and only a group or a folder (`search_only::add`). | ✅ |
| 4.5 | The inspector header says "only in search" where a tray menu says "tray icon"; the delete confirmation and tooltip say so too. | 👁 |

**Done when:** 👁 the spec §2.2 example's Admin tools and Scripts, built from
the screen, match the XML in the manual.

### 11.3.5 — Edit and Add-to-menu from y-pop

| | Task | Status |
|---|---|---|
| 5.1 | `ipc::path_of` returns `s/<n>/…` for a search-only item; `roots::resolve_path` and `select_path` understand it and open its page; `--select search` opens the Search page. | ✅ |
| 5.2 | Not done: Search's Edit on a section row and the free-key `CheckKey` still open the app without selecting (the hotkey could be `--select search`). | ⬜ |
| 5.3 | Tests in `ipc`, `roots` and `menu_tests`; the user manual's Edit and Configuring-the-search texts updated. | ✅ |

### 11.3.6 — One way to choose a key (D1, D4, B9)

Today the picker cannot simply be called from the Search page. `MenuMsg::Hotkey*`
stop at `open_root()`, which is `None` there; `hotkey_modal` and `HotkeyUi` are
addressed by `(ItemId, index)`; and the picker offers what an action cannot use.
`Chord` has no Win, and only letters, digits, F1–F12, Enter and Del (*probe*:
`Ctrl+Space`, `Ctrl+Up`, `Ctrl+F13`, `Win+E`, `Ctrl+Numpad5` are all refused).

| | Task | Status |
|---|---|---|
| 6.1 | `ui-kit`: `key_picker_with` takes `KeyRules` (which modifiers, which keys) and a `problem` line; `key_picker` is unchanged for `y-snap-config`. Categories and search hits with no allowed key are left out. Never imports a `Chord`. | 👁 |
| 6.2 | New `key_edit.rs`: `KeyTarget` (`Item(id, n)`, `SearchHotkey`, `Action(n)`, `MenuKey`), `Msg::Key(KeyMsg)` at app level, one `key_modal` replacing `inspector::hotkey_modal`. Item hotkeys move onto it. | 👁 |
| 6.3 | **A draft, not a live edit:** the combination stays in the modal until OK; Cancel touches nothing. A live edit would write out all of `<actions>` for a modal that is then cancelled, and puts half a combination in the model. It removes the restoring logic of `HotkeyClosed`; the keycaps behind the modal stop following along, which is the price. | 👁 |
| 6.4 | Action rules: `Chord::parse` of the draft is the only validation (no copied rules), its error the `problem` line and OK disabled. The file gets `Chord::to_string` (`Ctrl+Del`). A key taken by another action or the menu key is a problem too, naming it. | 👁 |
| 6.5 | Search hotkey card: a `key_field` (keycaps on a button, ✕); the Override switch only when there is a key, and Remove clears it (B9). Text the file holds that can't be read shows with "y-pop ignores it" and stays until changed. | 👁 |
| 6.6 | Conflicts: one `key_conflict(settings, text, target)` over items **and** the search hotkey, used by both pages ("Also used by Search"), plus `keycheck::reserved_for` ("Windows keeps this for Search"). A warning, not a refusal, as for items. | 👁 |
| 6.7 | Actions table: a `key_field` in the Shortcut column; the menu key the same way. `SearchMsg::Hotkey(String)`, `ActionKey(_, String)` and `MenuKey(String)` go. | 👁 |

**Done when:** no key anywhere in the app is a text box, and a test shows Cancel
leaves `to_xml()` identical, on all four targets.

### 11.3.7 — Logic fixes (no UI, a failing test first)

| | Task | Status |
|---|---|---|
| 7.1 | B1: deleting a tray menu drops it from `include`, in the same Undo step; an `include` left empty is removed, which means every menu. | ✅ |
| 7.2 | B2, B4: the rename follows the *menu*, not the text. Remember the last usable name per renamed item, so `Work` → `` → `Job` still replaces `Work`. A `;` says so under the name field, and the entry follows once the name is valid again. | ✅ |
| 7.3 | B3: `Include` writes the names it can and says which it can't: only the `;` menu is unavailable, the others stay clickable. | ✅ |
| 7.4 | B5: `y_pop::settings::Settings::all_items()` (tray and search-only), used at the four call sites. `items()` stays tray-only for hotkey conflicts: y-pop ignores a hotkey in a search-only tree. | ✅ |
| 7.5 | B7: `sections()` and `actions()` follow the reader's drops. **As built:** `Row.ignored`, `ActionRow.ignored`; the two pinning tests stay, with a new one on the awkward files. Rows y-pop drops are shown as such, removable, never editable. | ✅ |
| 7.6 | B8, B10, B11, B12: warn when nothing holds Enter; no box maximum below what the file allows; removal takes the ignored duplicates with it; `move_section` reports what it did. | ✅ |

**Done when:** each bug above has a test. **As built:** the bugs were shown by the review's probes, and the tests were written with the fixes, not before them.

### 11.3.8 — The page, by what y-pop does (D2, D3, D5–D8, B6)

Replaces the sections table and reorders the page. In the toolkit's words:

```
Search
 Opening      Hotkey  [Alt][Space]  Change  Remove
                      Override · "Also used by …"
              Open collapsed
 Empty box    Recent  [on]  How many [5]     ← off with history: says why
              Browse list (a table, ↑↓ only here)
                Tray menus    every menu, as columns     Edit
                Admin tools   group of 4               Edit  ✕
                [Add group]              [Reset to defaults]
 Results      Results · Buttons on the selected result · Use Windows history
 Also searched
              Folders   Scripts (with /)               Edit  ✕   [Add folder]
              Start menu  [off]  not used by y-pop yet
              Plugins   P (kept, not run)                    ✕
 Actions      Menu key · Quick pick · the table · Add · Reset
 Turn search off  (asks when it would lose work)
```

| | Task | Status |
|---|---|---|
| 8.1 | `Screen::Search` laid out as above. Rows keyed by identity (D8). "Open" a row is one click (D7). | 👁 |
| 8.2 | **Recent** is a switch and a count: off removes `<recent>` (B6). Disabled, with the reason, while history is off (D3). | 👁 |
| 8.3 | **Browse list**: Tray menus and search-only groups only, reordered among themselves. `move_section` must keep y-pop's order rules; the others keep their place in the file. | 👁 |
| 8.4 | **Tray menus'** fields (Opens as, which menus count) open from its row's Edit. | 👁 |
| 8.5 | **Also searched**: folders, Start menu, plugins, no arrows. Their file order stays. | 👁 |
| 8.6 | **Back to the default sections**, with Undo. **Changed:** it keeps search-only items and plugins, writing the three out before them, so nothing needs confirming. No more "(default)", "written" or `<tag>` wording on the page (D5). | 👁 |
| 8.7 | The Button column is disabled, with the reason, while row buttons are off (D3). | 👁 |
| 8.8 | **Turning search off** asks first when it would remove groups, folders or edited actions, naming them. **Changed:** still a button, not an On switch at the top: a switch should not be what deletes this much. | 👁 |
| 8.9 | B13, and `user-manual.md`, `ui-checklist.md` and the plan brought up to date. | 👁 |

**Done when:** every control changes something a person can see in y-pop, and
the first "Done when" of 11.3.3 passes live.

### 11.3.9 — Live pass

| | Task | Status |
|---|---|---|
| 9.1 | The user runs the `ui-checklist.md` Search table, at 560 px and maximized, against a real y-pop. Anything found goes back into this plan. Only then does 11.3 leave ⬜ in `search-plan.md`. | ⬜ |

## Risks

- **Generalising `tree.rs` (11.3.1) is the one risky step**: every menu edit
  goes through it. Do it first, behind the unchanged tests, and keep `Location`
  additions backward compatible so `menu_edit` needs no change beyond the
  top-level list.
- **The "one section written turns the defaults off" rule** must be the
  model's, not the view's: `sections()` returns what y-pop will use, so the
  screen can never show a list different from the behaviour. A test pins it to
  `y_pop::config`'s own reading of the same file.
- **Reordering mixes kinds.** `children` interleaves comments, sections and
  items; the user-visible order is the order of sections and search-only
  items together. Comments stay attached to what they precede
  (`lead`), as for tray items.
- **Layout is verified by reading, not by running** (`ui-toolkit.md`): the
  tables and the key picker modal are the parts to check live.

## Order and size

11.3.1 and 11.3.2 are logic with tests and need no window — about half the
work, and the part to get right. 11.3.3 is the new view, reusing `Table`, the
key picker and `labeled_switch`. 11.3.4 is mostly plumbing. 11.3.5 is small.
A useful stopping point exists after 11.3.3: search can be configured, only
search-only items still need the file.

Remaining: the live pass (11.3.9), then 5.2.
