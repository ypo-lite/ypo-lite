# Keys and layers: a group owns what it controls

A change to how `<controls>` is organised in `y-coustic`, and what it asks of
`y-coustic-config`. It is small enough for one plan; it does not change the
design in [`y-coustic-plan.md`](y-coustic-plan.md) beyond the table in
*Controls*, which this file replaces.

Status: ⬜ todo · 🔨 in progress · ✅ done · 👁 done, verify live

## The problem

Today `<controls>` is a product: every `<keys>` group is crossed with **every**
`<layer>`.

```xml
<controls>
  <keys up="VolumeUp"   down="VolumeDown" mute="VolumeMute"/>
  <keys up="Win+NumAdd" down="Win+NumSub" mute="Win+Numpad0"/>

  <layer mod="none"  target="master"/>
  <layer mod="Shift" target="mic"/>
  <layer mod="Ctrl"  target="focused"/>
</controls>
```

That is right for the keyboard's own volume keys: one set of keys, and the
modifier you hold says what they control. It is wrong as soon as there are
several sets of keys, because the layers are shared:

* A set of keys that **only** handles the microphone (a `Win+Alt+M` mute, an up
  and a down of its own) cannot exist: it would also get the master, the
  focused-application and the pointed layers, on the same keys with other
  modifiers.
* The same for a set that only drives the **focused application**.
* Two sets of keys cannot have different layers (the numpad wanting `Alt`
  where the media keys want `Ctrl`).

The single `<hotkey>`s can do one-off things (`target="mic"`), but only one key
for one action each; a pair of up and down keys for the microphone is two
entries, and there is no layer on them at all. And layers are of little use
there: a one-off key already says what it controls.

## The model

**A `<keys>` group owns how its keys are aimed, and there is nothing shared.**
One of two, written on the group itself:

| Scope | Written | Meaning |
| --- | --- | --- |
| **A target** | `target="mic"` | the keys control that target, with no modifier held. The shorthand for a set of keys that does one thing |
| **Layers** | `<layer>` children of the `<keys>` | the group is crossed with its own layers: the modifier held says what the keys control |

```xml
<controls>
  <!-- the keyboard's volume keys: the modifier says what they control -->
  <keys up="VolumeUp" down="VolumeDown" mute="VolumeMute" plain="windows">
    <layer mod="none"  target="master"/>
    <layer mod="Shift" target="mic"/>
    <layer mod="Ctrl"  target="focused"/>
  </keys>

  <!-- a mute of the microphone, and nothing else -->
  <keys mute="Win+Alt+M" target="mic"/>

  <!-- the focused application, on keys of their own -->
  <keys up="Win+Alt+PageUp" down="Win+Alt+PageDown" target="focused"/>
</controls>
```

**What stays.** `up`, `down`, `mute` (at least one) and `plain` on a group. The
rules of `expand`: `plain="windows"` leaves the bare media keys to Windows, a
media key is hook-only, a combination used twice keeps the first use and warns,
an `Alt` layer warns. `<hotkeys>` stays as it is, for what a group cannot say
(`set-volume`, `mixer`) and for one-off keys; it has no layers.

**Reading.**

* `target` and `<layer>` children on one group: the layers win and the target is
  warned about as ignored.
* A group with neither controls nothing: a warning, and it is kept in the file.
* An unknown `target` is a warning, and the group controls nothing until fixed.
* A `<layer>` directly under `<controls>`, as the files written so far have, is
  no longer read: it is warned about ("a `<layer>` belongs inside a `<keys>`")
  and kept in the file as a comment holding its own XML, as every element the
  reader cannot use already is. Nothing is released, so there is no
  compatibility mode; the shipped default is rewritten, and a hand-written file
  needs its layers moved into its groups (the warning says so).

**The warnings** name the group ("the keys VolumeUp / VolumeDown / VolumeMute")
so a collision can be found when there are several groups.

## Decisions

1. **No shared layers.** A shared default would make the XML shorter and the
   editor worse: a group's card would depend on a section elsewhere, editing
   that section would silently change several groups, and there would be a third
   state to explain. Repeating layers is what the editor is for: *Duplicate
   group* copies a group with its layers, and *Copy layers from…* copies them
   from another group onto this one.
2. **`target` on a group is the shorthand for the common case**, a set of keys
   that controls one thing. It is a layer with no modifier, so it turns into
   own layers without changing what it does.
3. **`<hotkeys>` stays and gets no layers.** A group with a `target` covers up,
   down and mute for anything; `<hotkey>` remains for `set-volume` and `mixer`,
   and for the key someone wants to read in one line. The docs say which to
   prefer.
4. **No new action on a group.** A group is up, down, mute: what a layer makes
   sense for. Anything else is a `<hotkey>`.
5. **The Keys screen has a card per group**: its three keys, then *Controls* as
   two choices (*One target*, *Layers by modifier*), then the target combo or
   the layers table. No section outside the groups.

## Phases

### A: `y-coustic`

| # | Task | Status |
| --- | --- | --- |
| A1 | `controls.rs`: `KeysGroup` gains `scope: Scope` (`Target(Target)`, `Layers(Vec<Layer>)`); `Controls` loses its `layers`; `KeysGroup` is no longer `Copy` | ✅ `Scope` (`Target`, `Layers`), `KeysGroup::label`, `Controls` has only `keys`. |
| A2 | `config.rs` reader: `target` on `<keys>`, `<layer>` children, the warnings above; a top-level `<layer>` is warned about and kept as a comment; `<keys>` is walked with its comments like a list (B1) | ✅ a nested `walk` for a group's layers; a top-level `<layer>` is warned about and kept as a comment through the existing machinery. |
| A3 | `controls::expand`: each group is crossed with the layers its scope gives; `Alt` and duplicate warnings name the group | ✅ `expand` also warns for a group that controls nothing. |
| A4 | Tests: both scopes, a target-only microphone mute beside the media group, a target and layers on one group, a group with neither, a top-level layer warned about and kept | ✅ 14 tests in `controls.rs`, the reader tests in `config.rs`. |
| A5 | `rsc/y-coustic.xml`: the default keeps its meaning with the layers inside the media group (and the numpad group, as a target or with its own); the comment shows a target-only group | ✅ the default writes the media group's four layers and the numpad group's four, as the shared four meant; `docs/examples/y-coustic.xml` shows a target. |
| A6 | Docs: `configuration.md` (`<controls>`, and what to change in an older file), the user manual, the design's *Controls* table, `development.md` | ✅ configuration, user manual, development notes, the design's *Layers* and sketch. |

### B: the round trip, in the library

| # | Task | Status |
| --- | --- | --- |
| B1 | `kept`: a group's layers have `Part`s of their own, and a group has a tail for comments after its last layer (`Kept::group_layers`, `Kept::group_tails`, parallel to `controls.keys`); `Kept::layers` goes | ✅ `kept::GroupPart { part, layers, tail }` per group, `Kept::keys`; `Kept::layers` is gone. |
| B2 | `to_xml`: a group with layers is written with children; with a target, as an attribute | ✅ |
| B3 | `edit.rs`: insert, remove, move and duplicate of a group's layers, keeping the parts parallel; duplicate a whole group with its layers and their comments left behind; copy the layers of one group onto another; switching the scope keeps the other one for the session so choosing back loses nothing | ✅ `insert_group_layer`, `remove_group_layer`, `move_group_layer`, `set_group_scope`, `copy_group_layers`; groups move, remove and duplicate with their layers' parts. The *keep the other scope for the session* part is the editor's (C1). |
| B4 | Tests: comments in a group's layers survive a save, moves keep them, the shipped file still round trips and saves to the same bytes twice | ✅ |

### C: `y-coustic-config`

| # | Task | Status |
| --- | --- | --- |
| C1 | `edit.rs`: `GroupScope(group, ScopeKind)`, `GroupTarget(group, Target)`, the group's layer edits (a modifier toggled, a target, add, move, remove with Undo), *Duplicate group*, *Copy layers from…*; the shared-layer edits go | ✅ `GroupScope`, `GroupTarget`, `GroupLayerModifier`/`Target`, `AddGroupLayer`, `MoveGroupLayer`, `RemoveGroupLayer`, `DuplicateGroup`, `CopyLayers` (13 edit tests). Switching to a target keeps the first layer's target and has an Undo when it drops layers; switching to layers makes the target the one bare layer, so no session stash is needed. |
| C2 | `keys_view.rs`: a card per group (keys as now; *Controls* as two choices; the target combo or the layers table under it; move, duplicate, delete); *Add a group* asks for a first key, as now | 👁 `keys_view::group_card`: keys in an `AutoGrid`, *Controls* as two choices, the target combo or `layers_block` (table, *Add a layer*, *Copy the layers of group N* as buttons because a combo would keep showing its last choice); *Duplicate* and *Delete* in the card's header. |
| C3 | The readout names the group a binding comes from; the Alt warning sits by the layer's row if cheap, else stays in the readout | 👁 the warnings name the group; the readout does not name a binding's group (`expand` returns bindings only), and the Alt warning stays in the readout. |
| C4 | The checklist's Keys table follows; the config plan's Phase 5 notes point here | ✅ |

### D: live

The user checks: the new default behaves as the old one did; a microphone-only
mute group works beside the media group; a focused-application up and down pair
works; the editor shows and changes both scopes and saves a file `y-coustic`
loads without a warning.

## Order and size

A and B are library work with tests and no window; C is the screen. A first (it
fixes what is meant), then B (so the editor can save it), then C. Unlike a
purely additive change, A breaks files with top-level layers: the user's own
`y-coustic.xml` needs its layers moved into its groups, and the warning says
where. `y-coustic-config` as it is now edits the old shape, so C follows A and B
closely: between them the editor would still show the shared layers it can no
longer save into the file, which is why A, B and C are meant to be merged
together.
