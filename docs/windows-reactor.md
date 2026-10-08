# `windows-reactor` 0.100.0 reference

Internal reference for working on `y-snap-config` (or any future WinUI 3 app in this
repo) built on the `windows-reactor` crate. Compiled by reading the crate source at
`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/windows-reactor-0.100.0/src/`
so a future session doesn't have to re-read ~24k+ lines of it from scratch. Every claim
below is backed by a specific file/line in that source; where the crate simply doesn't
have something, that's stated explicitly instead of guessed at.

Source files referenced: `lib.rs`, `element.rs`, `generated.rs` (~19.3k lines, the
control builder API), `reference.rs`, `app.rs`, `core/*.rs` (`component.rs`,
`engine.rs`, `scheduler.rs`, `runtime.rs`, `scope.rs`, `virtual_model.rs`, `arena.rs`,
`keyed.rs`), `native/winui/*.rs` (`mod.rs` ~4.3k lines, `grid.rs`, `content_dialog.rs`,
`element_factory.rs`, `bootstrap.rs`, `framework.rs`, `generated.rs`,
`bindings.rs` — the raw WinRT metadata bindings, auto-generated and not hand-written API).

---

## 1. Core architecture

### The `Component` trait

Defined in `core/component.rs`. This is the Elm-like unit of state:

```rust
pub trait Component: Sized + 'static {
    type Input: Clone + PartialEq + 'static;
    type Message: 'static;

    fn create(input: &Self::Input, context: &ComponentContext<Self>) -> Self;
    fn input_changed(&mut self, _input: &Self::Input, _context: &ComponentContext<Self>) {}
    fn update(&mut self, _message: Self::Message, _context: &ComponentContext<Self>) {}
    fn view(&self, input: &Self::Input, context: &mut ViewContext<Self>) -> View;
}
```

- `create` runs once when the component is first reserved.
- `input_changed` runs when a parent supplies a new `Input` that is `!=` the current one
  (components are reconciled like React: same component type at the same tree position
  and equal `Input` reuses state, changed `Input` calls `input_changed`, a different
  component type or removed position tears the old one down).
- `update` runs once per accepted queued message (see `LocalSender` below) — never
  called inline/synchronously from a callback.
- `view` runs to (re)publish the tree; it also records context reads, effects, and
  window-level declarations (title, visuals, size/color-scheme observers) via
  `ViewContext`.

A component is mounted into a tree with `View::component::<C>(input)`.

### `ComponentContext` (create/update/input_changed)

```rust
pub struct ComponentContext<C: Component> {
    sender: LocalSender<C::Message>,
    tasks: TaskSpawner,
    window: WindowRef,
}
impl<C: Component> ComponentContext<C> {
    pub fn open_window(&self, root: View) -> bool;
    pub fn sender(&self) -> LocalSender<C::Message>;
    pub fn spawn_background<F>(&self, work: F) -> ComponentTask
        where F: FnOnce(CancellationToken) -> C::Message + Send + 'static;
    pub fn spawn_background_with_rejection<F>(&self, work: F, rejected: C::Message) -> ComponentTask;
    pub fn window(&self) -> WindowRef;
}
```

`spawn_background` submits work to the Windows thread pool (`windows_threading::submit`);
its result is delivered as a queued `Message`, never by directly mutating state from
another thread. `WindowRef::request_close()` / `open_window` are accepted only while a
lifecycle call (`create`/`input_changed`/`update`) is actively running.

### `ViewContext` (view)

```rust
impl<C: Component> ViewContext<C> {
    pub fn on_color_scheme(&mut self, callback: impl IntoPayloadCallback<ColorScheme>);
    pub fn on_window_size(&mut self, callback: impl IntoPayloadCallback<WindowSize>);
    pub fn window_title(&mut self, title: impl Into<String>);
    pub fn window_visuals(&mut self, visuals: WindowVisuals);
    pub fn sender(&self) -> LocalSender<C::Message>;
    pub fn callback<T>(&self, map: impl Fn(T) -> C::Message + 'static) -> Callback<T>;
    pub fn forward(&self) -> Callback<C::Message>;
    pub fn message(&self, message: C::Message) -> Callback<()> where C::Message: Clone;
    pub fn use_context<T: Clone + 'static>(&mut self, context: &Context<T>) -> T;
    pub fn use_effect<D: PartialEq + 'static>(
        &mut self, key: impl Into<EffectKey>, dependency: D,
        setup: impl FnOnce() -> Option<Box<dyn FnOnce()>> + 'static,
    );
}
```

`context.callback(|payload| Message::Variant(payload))` is the idiomatic way to turn a
control event into a message — it returns a `Callback<T>` that queues
`Message::Variant(payload)` via the component's `LocalSender` when called; it never runs
`update` synchronously. Declaring `window_title`/`window_visuals`/the two `on_*`
observers more than once per `view()` call is a hard error
(`ComponentDeclarationError::WindowTitle` etc., turned into a `PumpError` at the Pump
level) — each may be declared at most once per publication.

`use_effect` is the escape hatch for imperative setup/cleanup keyed by a dependency
value (like React's `useEffect`): setup reruns only when `dependency` changes vs. the
last publication, and its returned cleanup runs before replacement or on removal.

### Bootstrapping: `App::run_component`

`app.rs`:

```rust
pub fn run_component<C: Component>(input: C::Input) -> windows_core::Result<()> {
    Self::run(View::component::<C>(input))
}
```

This wraps `View::component::<C>(input)` as the tree root, creates one `Pump<WinUiRuntime>`
per window, mounts it on `Application::Start`'s `on_launched` callback, and blocks the
calling thread in the WinUI message loop until every window closes. `App::run` (one
window), `App::run_windows` (N independent top-level windows, each with its own `Pump`),
and `App::run_component` are the only three public entry points.

### The message pump / scheduler, at the level you need

Three cooperating pieces (`core/scheduler.rs`, `core/pump/*.rs`, `core/component.rs`):

1. **`SchedulerState`** (`core/scheduler.rs`) is a tiny state machine — `Idle` /
   `Scheduled(ticket)` / `Dispatching` / `Closing` — that coalesces repeated
   "something changed, please redraw" requests into a single pending dispatch, with a
   `WorkPriority::{Low, Normal}` so a higher-priority request can preempt/upgrade an
   already-scheduled lower one. It is what turns many `LocalSender::send` calls in one
   tick into a single dispatcher-queue callback rather than N.
2. **`ComponentStore::drain(budget)`** (`core/component.rs`) pops queued local messages
   and background-task results (round-robining between the two queues) and calls
   `Component::update` for each, up to `budget` per drain — this bounds how much work one
   scheduler tick does.
3. **The `Pump`** (`core/pump/mod.rs`) owns the `ComponentStore`, the reconciled native
   `Tree`, and the `NativeRuntime` (WinUI, in `native/winui/mod.rs`). After draining
   messages it re-runs `view()` for every dirty component, diffs the resulting `View`
   tree against the previous one, and emits native property-set / insert / remove
   commands to the runtime.

**What triggers a re-render**: calling `.send()` on a `LocalSender<M>` (directly or via
a `Callback` built from `context.callback(...)` / `ViewContext::forward` /
`ComponentContext::sender()`), a delivered background task result, or a context value
changing on a provider that this component's `view()` previously read via
`use_context`. Each of these marks the component's `ComponentToken` dirty and requests a
scheduler dispatch; nothing re-renders synchronously inside the call that triggered it.

---

## 2. `View` and `element.rs`

### `View` is a closed enum, not a trait object

`element.rs` (`ViewKind`):

```rust
pub struct View(ViewKind);
enum ViewKind {
    Native(Element),
    Component(ComponentView),
    Fragment(Rc<Vec<KeyedView>>),
    Provider { provision: ContextProvision, child: Box<Self> },
    Content { control: Element, content: Box<Self> },
    Children { control: Element, children: Rc<Vec<KeyedView>> },
    Slots { control: Element, slots: Rc<Vec<SlottedView>> },
    Tooltip { .. }, Flyout { .. }, Menu { .. }, CommandBarFlyout { .. },
    TreeNodes { .. }, ContentDialog { .. },
}
```

A concrete builder (`Button::new()`, `Grid::new()`, ...) is a generated struct in
`generated.rs`; it becomes a `View` only via `.into()` (`impl From<Element> for View`)
or by calling a trait method that itself returns `View` (`.content(...)`,
`.children(...)`, `.slots(...)`). Every control implements `sealed::NativeControl`
(`into_element`), and most implement `sealed::LayoutControl` (exposes a mutable
`Option<Rc<ElementState>>` slot used by `LayoutControl`/`GridChildExt`/etc. — see §3).

### Composition traits

- **`ContentControl`** — single-child controls (`Button`, `Border`, `ScrollViewer`, ...):
  `.content(view)`.
- **`ChildrenControl`** — multi-child panels (`Grid`, `StackPanel`, `VariableSizedWrapGrid`):
  `.children(...)` (static/positional, via the sealed `IntoViews`/tuple impls) or
  `.keyed_children(...)` (dynamic, reconciled by `Key`).
- **`SlotsControl`** — composite controls with named, typed slots (`NavigationView`,
  `ComboBox`'s header, `ToggleSwitch`'s on/off content, `Expander`'s header/content,
  `SelectorBar`'s items, ...):
  ```rust
  pub trait SlotsControl: ... {
      type Slot: Copy;
      fn slot(self, slot: Self::Slot, view: impl Into<View>) -> View;
      fn collection_slot<T: Into<KeyedView>>(self, slot: Self::Slot, children: impl IntoIterator<Item = T>) -> View;
      fn slots(self, slots: impl IntoIterator<Item = SlotView<Self::Slot>>) -> View;
  }
  ```
  Each control's `Slot` enum lists exactly what native slot exists — e.g.
  `NavigationViewSlot::{Content, Header, PaneCustomContent, PaneFooter, MenuItems}`,
  `ExpanderSlot::{Header, Content}`, `ToggleSwitchSlot::{Header, OnContent, OffContent}`,
  `ComboBoxSlot::Header`, `SelectorBarSlot::Items`, `SelectorBarItemSlot::Icon`. A slot
  is filled with `SlotView::new(slot, view)` (single view) or
  `SlotView::collection(slot, keyed_items)` (a reconciled collection, used e.g. for
  `NavigationViewSlot::MenuItems` or `SelectorBarSlot::Items`).

### Fragments, keys, `IntoViews`

`View::fragment(children)` takes a **statically shaped** expression — `()`, a
fixed-size array `[T; N]`, or a tuple up to 16 elements (`impl_into_views_tuple!` macro)
— reconciled by *position*. A `Vec<View>` or an iterator adapter is explicitly rejected
at compile time (`element.rs` has `compile_fail` doctests proving this) — dynamic
collections must go through `View::keyed_fragment(items)` /
`ChildrenControl::keyed_children(items)` / `SlotsControl::collection_slot(...)`, all of
which reconcile by `Key` (`Key::from(u64|u32|usize|String|&str)`).

### `GridChildExt` / `CanvasChildExt` / `RelativePanelChildExt` need a native wrapper

Because a `Component` or `Fragment` can expand to more than one native root, per-cell
placement methods (`.grid_row(0)`, `.canvas_left(4.0)`, relative-panel alignment) are
only implemented for `LayoutControl` (concrete native controls), not for `View` itself
— `element.rs` has `compile_fail` doctests showing `View::component::<Child>(()).grid_row(0)`
and `View::fragment((...)).grid_column(0)` both fail to compile. **Wrap a composed
child in a native control (e.g. a `Border` or a single-child `Grid`) before placing it
in a parent `Grid`.**

---

## 3. Layout primitives — and exactly how each one sizes itself

This is the crux of the shared-column-width bug. Read this whole section before
touching swatch/label layout.

### `Thickness` / `CornerRadius`

Four DIP values (`Thickness::uniform`, `::xy(h, v)`, `::new(l, t, r, b)`); both assert
finite + non-negative when consumed by the controls that accept them (`Border.padding`,
`.border_thickness`, `.corner_radius`, etc.).

### `GridLength`

```rust
pub enum GridLength { Auto, Pixel(f64), Star(f64) }
impl GridLength { pub const STAR: Self = Self::Star(1.0); }
```

`Grid::rows(...)`/`.columns(...)` take `impl IntoIterator<Item = GridLength>` and
**assert** every value `is_valid()` (finite, `>= 0.0`, or `Auto`) — an invalid length
panics at view-construction time, it does not silently clamp.

### `Grid` — the only control with row/column tracks

`generated.rs` (`Grid` struct, `native/winui/grid.rs`):

```rust
pub struct Grid {
    row_spacing: Property<f64>,
    column_spacing: Property<f64>,
    rows: Property<Rc<Vec<GridLength>>>,
    columns: Property<Rc<Vec<GridLength>>>,
    // ...
}
```

Each child is placed with the `GridChildExt` methods `.grid_row(i)` / `.grid_column(i)`
/ `.grid_row_span(n)` / `.grid_column_span(n)` (all on `ElementState`, defaulting to
`Option::None` = row/column 0, span 1 — same as XAML's `Grid.Row`/`Grid.Column`
defaults). `native/winui/grid.rs::set_definitions` maps `GridLength::{Auto, Pixel,
Star}` straight onto `bindings::GridUnitType::{Auto, Pixel, Star}` and
`RowDefinition`/`ColumnDefinition` — this is literally XAML Grid semantics with no
extra behavior layered on:

- **`Auto`** column/row: sized to the *widest/tallest content that lands in that
  track*, across every child assigned to it in that one `Grid`.
- **`Pixel(n)`**: fixed DIPs, content is clipped/overflows if it doesn't fit.
- **`Star(n)`**: `n` shares of whatever DIPs remain after `Auto` and `Pixel` tracks are
  subtracted from the `Grid`'s own width — the "remaining space" is divided among
  `Star` tracks *in proportion to their `n`*, and if there is no remaining space (or
  the `Grid` itself is only as wide as its `Auto`/`Pixel` content demands, i.e. the
  `Grid`'s own width is unconstrained/auto), `Star` tracks shrink toward zero. `Star`
  columns **squeeze** rather than reflow — see §5.

**Column/row sizing is strictly per-`Grid`-instance.** There is no cross-`Grid`
mechanism in this crate — see the dedicated subsection below.

### `StackPanel` — sized to content along its stacking axis, and only there

```rust
pub struct StackPanel {
    orientation: Property<Orientation>,
    spacing: Property<f64>,
    // ...
}
```

A `StackPanel` measures **Auto** (content-sized) along `orientation`
(`Orientation::{Horizontal, Vertical}`, default vertical — same XAML default): a
vertical stack's height is the sum of its children's heights plus `.spacing(n)` gaps; a
horizontal stack's width is the sum of children's widths plus gaps. **Across the
cross-axis it does not constrain children at all** — each child gets whatever width (for
a vertical stack) it wants unless the child itself sets `HorizontalAlignment::Stretch`
and the `StackPanel`'s own width is constrained by *its* parent. `StackPanel` has no
`Width`/`MinWidth` builder methods of its own beyond the generic `LayoutControl` ones
(`.width()`, `.min_width()`, etc., which set the panel's *own* box, not its children's).

**This is very likely the direct mechanism behind the "fields size themselves from
their own content" bug**: if each labeled field is its own `StackPanel`/`Grid`
sized `Auto` to its label text, and sibling fields are separate instances (not columns
of one shared `Grid`), nothing in this crate makes their widths agree — see below.

### `ScrollViewer`

```rust
pub struct ScrollViewer {
    horizontal_scroll_bar_visibility: Property<ScrollBarVisibility>,
    vertical_scroll_bar_visibility: Property<ScrollBarVisibility>,
    // ...
    content: Option<Box<Element>>,
}
```

Single-content (`ContentControl`), `.horizontal_scroll_bar_visibility(...)` /
`.vertical_scroll_bar_visibility(...)` take `ScrollBarVisibility` (WinUI's
`Auto`/`Visible`/`Hidden`/`Disabled`, generated in `generated.rs`). There is also a
separate, newer `ScrollView` control (`ScrollingScrollBarVisibility` instead of
`ScrollBarVisibility`) — both exist side by side in this version; `ScrollView` is the
WinUI 3 `Microsoft.UI.Xaml.Controls.ScrollView` (successor to `ScrollViewer`), so prefer
it in new code unless something specifically needs `ScrollViewer`. Neither type imposes
any column/row-sharing behavior; they simply clip+scroll their one child.

### `VariableSizedWrapGrid`

```rust
pub struct VariableSizedWrapGrid {
    item_width: Property<f64>,   // must be finite & > 0 if set
    item_height: Property<f64>,  // must be finite & > 0 if set
    orientation: Property<Orientation>,
    // ...
}
```

Lays children out in a uniform grid of `item_width` × `item_height` cells and **wraps**
to a new row/column when it runs out of space in the container's cross-axis — this is
the crate's one built-in *reflow* container (see §5). `item_width`/`item_height` are
per-panel constants (every cell the same size), not per-column — there is no way to
give this panel different per-column widths the way `Grid` can.

### `HorizontalAlignment` / `VerticalAlignment` / `Width` / `Min`/`MaxWidth` — what they do and don't do

All defined on `ElementState` (`element.rs`) and exposed via the `LayoutControl` trait,
which is implemented for essentially every control:

```rust
pub enum HorizontalAlignment { Left, Center, Right, Stretch }
pub enum VerticalAlignment { Top, Center, Bottom, Stretch }
```

- `.width(n)` / `.height(n)` set an **explicit** box size (finite, `>= 0.0`, asserted).
  This pins that element's own size regardless of alignment.
- `.min_width(n)` / `.max_width(n)` / `.min_height(n)` / `.max_height(n)` clamp the
  *measured* size — they don't force two unrelated siblings to the same width unless
  you compute and pass the *same* literal number to both.
- `.horizontal_alignment(Stretch)` makes the child fill whatever width its **immediate
  parent already handed it** (a `Star`/fixed `Grid` column, a `Border`'s content area,
  etc.) — it does **not** reach further up the tree and does **not** make siblings
  match each other's width if they live in different parents (e.g. two different
  `StackPanel`s, or two different `Grid`s).
- Every one of these is a `Property<T>` defaulting to `Inherited` (`element.rs`,
  `Property::Inherited` is the `#[default]`) — unset means the property is left unset
  on the native `FrameworkElement`, so **the control's own default WinUI style governs**
  (e.g. most `Control`-derived elements default `HorizontalAlignment` to `Stretch`
  already; `TextBlock`/`Image` etc. default to `Left`/`Top` per standard XAML). This
  crate does not override those native defaults itself — confirmed in
  `native/winui/mod.rs`, where `PropertyId::HorizontalAlignment` etc. map straight onto
  `FrameworkElement`'s own DP (`Self::Framework(runtime.ui_element(node)?)`).

**None of `Width`/`MinWidth`/`MaxWidth`/`Stretch`/`Star` equalizes sibling widths across
separate parents.** They only ever constrain size *within one measure/arrange pass
under one shared parent*.

### There is no `Grid.IsSharedSizeScope`/`SharedSizeGroup` equivalent in this crate

Confirmed by exhaustive search of `element.rs`, `generated.rs`, and
`native/winui/*.rs` for `SharedSize`/`IsSharedSizeScope` — **zero matches**. XAML's
`Grid.IsSharedSizeScope="True"` + `ColumnDefinition.SharedSizeGroup="X"` mechanism
(which synchronizes column widths across *separate* `Grid`s in the same scope) is not
exposed anywhere in `windows-reactor` 0.100.0, and nothing in `native/winui/grid.rs`
(which only ever calls `Grid::SetRow/SetColumn/SetRowSpan/SetColumnSpan` and
`RowDefinition::SetHeight`/`ColumnDefinition::SetWidth`) sets up such a scope
underneath. **If the underlying WinUI/XAML platform type itself supports
`IsSharedSizeScope`, this crate gives you no builder to reach it.**

**Practical fix for "fields/swatches size themselves from their own content instead of
a shared column" in an app built on this crate**: put every field (or every
label+swatch pair) in the **same** `Grid` instance as rows, with **one shared column
definition** (e.g. `columns([GridLength::Auto, GridLength::STAR])` for a
label-column/value-column layout), and assign each label `.grid_column(0)` and each
value `.grid_column(1)` with `.grid_row(i)` per field. Because `Auto` sizes to the
widest content *across the whole `Grid`*, every label column and every value column
will naturally agree, with no shared-size scope needed — the one-`Grid`-per-field
pattern is what breaks it, not the layout primitives themselves.

---

## 4. Common controls

All from `generated.rs`. `LayoutControl` (width/height/alignment/margin/grid-placement)
is implemented for every one of these unless noted.

| Control | Purpose | Key builders | Sizing/other quirks |
|---|---|---|---|
| `TextBlock` | Static text | `.text()`, `.text_wrapping()`, `.font_size()` (must be `> 0`), `.font_weight()`, `.max_lines()`, `.text_trimming()`, `.foreground()` | No slots. Sizes to its text (`Auto`) unless wrapped/constrained; `String`/`&str` also `impl Into<View>` directly (auto-wrapped in a plain `TextBlock`). |
| `TextBox` | Single/multi-line editable text | `.text()`, `.placeholder_text()`, `.accepts_return()`, `.text_wrapping()`, `.background()`/`.border_brush()`/`.border_thickness()`, `.on_text_changed()` | `TextBoxSlot::Header`. `FocusControl` (supports `ElementRef::request_focus`). No built-in numeric validation — that's what `NumberBox` is for. |
| `NumberBox` | Numeric input with min/max | `.minimum()`, `.maximum()`, `.value(Option<f64>)`, `.on_value_changed(Option<f64>)` | `NumberBoxSlot::Header`. `FocusControl`. Value is `Option<f64>` (NaN is filtered to `None` in `.value()`). |
| `ComboBox` | Dropdown selection from strings | `.items_source(impl IntoIterator<Item: Into<String>>)`, `.selected_index(Option<usize>)`, `.placeholder_text()`, `.is_editable()`, `.on_selection_changed(Option<usize>)` | `ComboBoxSlot::Header` only. Items are **plain strings** — no rich per-item view slot in this version, so an icon/subtitle per item is not directly supported (use `ListBox`/`GridView`/`NavigationView` items for rich content instead). `FocusControl`. |
| `ToggleSwitch` | On/off switch | `.is_on()`, `.on_toggled(bool)` | `ToggleSwitchSlot::{Header, OnContent, OffContent}` — the on/off captions are swappable views, not just strings. `FocusControl`. |
| `Button` | Push button | `.content()` (any `View`), `.on_click()`, `.horizontal_content_alignment()`/`.vertical_content_alignment()`, `.style(ButtonStyle)`, `.key_accelerators(KeyAccelerators)`, `.resource_overrides(ResourceOverrides)` | The **only** control in this crate with a `.resource_overrides()` builder (see §6/§7 — this is a real, source-confirmed gap, not a guess). `FocusControl`. |
| `ColorPicker` | Full WinUI color-picker panel | `.color(Color)`, `.is_alpha_enabled()`, `.is_hex_input_visible()`, `.is_color_slider_visible()`, `.is_color_channel_text_input_visible()`, `.on_color_changed(Color)` | No slots, no `resource_overrides`. Sizes to its own (fairly large) native content — not something you can compact via a wrapper `Grid` column, only via the visibility flags above. Not a `FocusControl`/`ReferenceControl` (no `ElementRef` support). |
| `SymbolIcon` / `Symbol` | Segoe Fluent icon glyph | `SymbolIcon::new().symbol(Symbol::Add)` | `Symbol` is a large closed enum of named MDL2/Fluent glyphs (`Add`, `Edit`, `Delete`, `Setting`, ... — see `generated.rs` lines ~159+). No arbitrary-codepoint variant; unlisted glyphs need `FontIcon::new().glyph("\u{E700}")` instead. |
| `NavigationView` / `NavigationViewItem` | App-shell nav pane | View: `.pane_display_mode()`, `.is_pane_open()`, `.open_pane_length()`, `.on_selected_tag_changed(Option<String>)`. Item: `.tag()`, `.is_selected()`, `.is_expanded()` | `NavigationViewSlot::{Content, Header, PaneCustomContent, PaneFooter, MenuItems}`; `NavigationViewItemSlot::{Content, Icon, MenuItems}` (items can nest via their own `MenuItems` slot). |
| `SelectorBar` / `SelectorBarItem` | Segmented-control-style tab strip | Bar: `.on_selected_text_changed(Option<String>)`. Item: `.text()`, `.is_selected()` | `SelectorBarSlot::Items` (keyed collection); `SelectorBarItemSlot::Icon`. |
| `Expander` | Collapsible section | `.is_expanded()`, `.on_is_expanded_changed(bool)` | `ExpanderSlot::{Header, Content}` — both are full `View` slots, not just strings. |

Other controls exist (`Slider`, `CheckBox`, `RatingControl`, `DatePicker`/`TimePicker`,
`CalendarDatePicker`, `ListBox`/`ListBoxItem`, `ListView`/`ListViewItem`,
`GridView`/`GridViewItem`, `Pivot`/`PivotItem`, `FlipView`, `TabView`, `TitleBar`,
`BreadcrumbBar`, `ProgressBar`, `AutoSuggestBox`, `PasswordBox`, `HyperlinkButton`,
`RepeatButton`, `Border`, `RelativePanel`, `SplitView`, `Canvas`, `WebView2`,
`SwapChainPanel`, `Image`/`ImageIcon`/`FontIcon`/`BitmapIcon`/`PathIcon`,
`ContentDialog`) — all in `generated.rs`, same builder-pattern shape.

---

## 5. Responsive / reflow patterns

Two genuinely different behaviors exist for "the window got narrower":

- **`Grid` with `Star` columns**: **squeezes**. Every `Star` column keeps its
  proportional share and everything keeps its assigned row/column — content that
  doesn't fit clips, wraps (if `TextWrapping` allows), or overflows; nothing moves to a
  new row. Good for "N columns that should always coexist side by side, shrinking
  together," bad for "cards that should wrap to fewer per row on a narrow window."
- **`VariableSizedWrapGrid`**: **reflows**. Fixed `item_width`/`item_height` cells wrap
  to a new row/column once the container's cross-axis runs out of room — the number of
  items per row changes with available width. Good for a swatch/card gallery; bad when
  you need differently-sized columns (it has only one uniform cell size for the whole
  panel).

There is no other adaptive/responsive layout primitive in this version (no
`WrapPanel`-with-variable-item-size, no breakpoint/`VisualStateManager`-driven layout
surface, no container queries) — `on_window_size` (`ViewContext::on_window_size`) is the
only hook for "do something different when the window is narrow," and it requires the
component itself to branch its `view()` output (e.g. switch from a `Grid` of `Star`
columns to a `VariableSizedWrapGrid`, or drop a column) based on the observed
`WindowSize`.

---

## 6. Styling / theming

### `WindowVisuals` / `WindowBackdrop` / `WindowTheme`

`element.rs`:

```rust
pub enum WindowBackdrop { None, Mica, MicaAlt, Acrylic }
pub enum WindowTheme { System, Light, Dark }
pub struct WindowVisuals { backdrop, client_size, constraints, icon, theme }
impl WindowVisuals {
    pub fn backdrop(mut self, backdrop: WindowBackdrop) -> Self;
    pub fn client_size(mut self, width: f64, height: f64) -> Self;   // both must be finite & > 0
    pub fn icon(mut self, path: &'static str) -> Self;               // must be non-empty
    pub fn constraints(mut self, constraints: WindowConstraints) -> Self;
    pub fn theme(mut self, theme: WindowTheme) -> Self;
}
```

Declared once per publication via `ViewContext::window_visuals(...)` — declaring it
twice in one `view()` is `ComponentDeclarationError::WindowVisuals`.
`WindowConstraints { min_width, min_height, max_width, max_height }` (all
`Option<f64>`) is validated (`min <= max`, all finite & positive) at
`.constraints(...)` call time, not deferred to runtime.

### `Color` / `Brush` / `ThemeBrush`

```rust
pub struct Color { pub a: u8, pub r: u8, pub g: u8, pub b: u8 }
pub enum Brush { Theme(ThemeBrush), Solid(Color) }
pub enum ThemeBrush {
    Accent, AccentText, PrimaryText, SolidBackground,
    CardBackground, CardStroke, SystemCritical, SystemCriticalBackground,
}
```

`ThemeBrush` resolves to a fixed, hardcoded set of eight WinUI theme resource keys
(`ThemeBrush::resource_key`, e.g. `Accent` → `"AccentFillColorDefaultBrush"`,
`CardBackground` → `"CardBackgroundFillColorDefaultBrush"`) — you cannot add your own
named theme brush key through `ThemeBrush`; anything else needs `Brush::Solid(Color)`
with a literal, non-theme-reactive color, or `ResourceOverrides` (next section).

### `FontWeight`

```rust
pub struct FontWeight(u16); // valid range 1..=999 (OpenType)
impl FontWeight {
    pub const THIN: Self; pub const EXTRA_LIGHT: Self; pub const LIGHT: Self;
    pub const SEMI_LIGHT: Self; pub const NORMAL: Self; pub const MEDIUM: Self;
    pub const SEMI_BOLD: Self; pub const BOLD: Self; pub const EXTRA_BOLD: Self;
    pub const BLACK: Self; pub const EXTRA_BLACK: Self;
    pub const fn new(weight: u16) -> Option<Self>; // None outside 1..=999
}
```

### `ResourceOverrides` — confirmed to be Button-only in this version

```rust
pub enum ResourceValue { Color(Color), Thickness(Thickness), CornerRadius(CornerRadius) }
pub struct ResourceOverrides { values: BTreeMap<String, ResourceValue> }
impl ResourceOverrides {
    pub fn set(mut self, key: impl Into<String>, value: impl Into<ResourceValue>) -> Self;
    // panics if key is empty, or a Thickness/CornerRadius is negative/non-finite
}
```

This is a **string-keyed XAML resource-dictionary override** (e.g. you could set
`"ButtonBackground"` or a corner-radius resource key some `Button` style reads), not a
generic style-property setter. Two hard limits, both directly confirmed in the source:

1. **Exactly one control exposes it.** A repo-wide search of `generated.rs` for
   `pub fn resource_overrides` returns exactly one hit: `Button::resource_overrides`
   (line ~531). No other control — not `ContentDialog`, not `Grid`, not `TextBox`, not
   any panel — has this builder at all, so there is currently no way to override *any*
   named resource on any other control type through this crate's public API.
2. **Its native plumbing is hardcoded to Button.** `native/winui/mod.rs` matches on a
   single dedicated property id, `PropertyId::ButtonResources`, whose set/clear paths
   (`set_resource_overrides`/`clear_resource_overrides`) install/remove entries in that
   one `FrameworkElement.Resources()` dictionary. There is no generic "any element's
   resource dictionary" property — the mechanism is Button-specific end to end, not
   just under-exposed.

**On `ContentDialog` specifically**: `ContentDialog`'s generated struct (`generated.rs`,
`pub struct ContentDialog`) has fields for `title`, the three button texts/enabled
flags, `on_closed`, `content`, and `is_open` — **no `resource_overrides` field and no
`.resource_overrides()` method at all**. So the claim that you can't override
`ContentDialog`'s native max-width (a WinUI default-style constant, e.g.
`ContentDialogMaxWidth`) through `ResourceOverrides` is confirmed structurally: the
builder to attempt it on `ContentDialog` does not exist in this crate version, full
stop — there's nothing to "not work," it's simply not wired up. If you need a wider
`ContentDialog`, your only lever from this crate is what `ContentDialog` itself
exposes (title/buttons/content) — sizing the dialog itself isn't exposed.

---

## 7. Known gaps / limits confirmed directly in the source

- **No keyboard-event hooks on any control.** Exhaustively grepping `generated.rs`
  for every `pub fn on_*` builder yields only: `on_click`, drag/drop
  (`on_drag_enter/over/leave`, `on_drop`), pointer events (`on_pointer_pressed/moved/
  entered/exited/released/capture_lost/canceled`), `on_item_clicked`,
  `on_text_changed`, `on_suggestion_chosen`, `on_password_changed`, `on_value_changed`,
  navigation/pane events, `on_toggled`, `on_is_checked_changed`, `on_checked`,
  `on_selection_changed`, `on_closed`/`on_opened`/`on_failed`, `on_color_changed`,
  date/time-changed events, `on_reordered`, `on_close_requested`,
  `on_add_tab_button_click`, `on_action_button_click`, `on_item_invoked`,
  `on_is_expanded_changed`, `on_back_requested`. **There is no `on_key_down`,
  `on_key_up`, `on_character_received`, or preview/tunneling key event anywhere** — a
  targeted grep for `KeyDown|KeyUp|CharacterReceived|PreviewKey|on_key` across
  `generated.rs` and `native/winui/mod.rs` (the hand-written public API and its native
  glue) returns zero matches. The only keyboard-adjacent facility is
  `KeyAccelerators`/`KeyAccelerator` (`element.rs`) — a fixed, small enum of
  accelerator keys (`AcceleratorKey::{R, NumberPad0..9, Divide, Multiply, Subtract,
  Add, Decimal, Enter}`) with `AcceleratorModifiers::{None, Control}`, attachable only
  to `Grid` and `Button` (`.key_accelerators(...)`) as global shortcuts, not a
  per-keystroke event stream. **This confirms the claim "windows-reactor exposes no
  keyboard-event hooks on any control" is still accurate for 0.100.0** — the only
  keyboard-adjacent surface is the closed accelerator-key list, and it does not cover
  arbitrary keys or give you raw key codes/characters.
  (Note: raw WinRT `KeyDown`/`KeyUp`/`CharacterReceived` events *do* exist as bindings
  in `native/winui/bindings.rs`, because that file is auto-generated WinRT metadata for
  the whole `UIElement`/`Control` surface — but nothing in the crate's hand-written
  public API (`generated.rs`, `element.rs`) exposes them as a builder method, so they
  are unreachable from application code.)
- **`ResourceOverrides` is Button-only** (see §6) — not a general per-element style
  hook, and not available on `ContentDialog`, `Grid`, or any panel/container.
- **No `Grid.IsSharedSizeScope`/`SharedSizeGroup` equivalent** (see §3) — sibling
  column/row widths cannot be synchronized across separate `Grid` instances by this
  crate's API.
- **`ComboBox` items are plain strings only** (`items_source: Rc<Vec<String>>`) — no
  per-item rich-content slot, unlike `ListBox`/`ListView`/`GridView`/`NavigationView`
  which take full keyed `View` collections.
- **`VariableSizedWrapGrid` has one uniform cell size** (`item_width`/`item_height` are
  panel-level, not per-item) — it cannot mix differently sized reflowing cells.
- **No container-query / breakpoint layout primitive** — adaptive layout is only ever
  "the component reads `WindowSize` via `on_window_size` and branches its own `view()`
  output," there's no declarative `VisualStateManager`-style trigger surface exposed.
- **Positional `View::fragment`/`.children(...)` explicitly reject dynamic
  collections at compile time** (`Vec<View>` and iterator adapters both fail via
  `compile_fail` doctests in `element.rs`) — always use the `keyed_*` variants for
  anything built from a runtime-length collection.
- **`GridChildExt`/`CanvasChildExt`/`RelativePanelChildExt` require a concrete native
  wrapper** — a `Component` or multi-root `Fragment` cannot be placed directly in a
  parent `Grid`/`Canvas`/`RelativePanel` cell (compile-time rejected); wrap it in a
  single native control first.

---

## Gotchas (sizing/layout footguns, collected)

1. **Two sibling fields in two different `StackPanel`/`Grid` instances will never
   agree on width automatically** — there is no shared-size mechanism in this crate.
   If widths must match, they must be columns of **one shared `Grid`** (or you must
   compute and apply the same literal `.width(n)`/`.min_width(n)` to both).
2. **`Auto` `Grid` columns size to the widest content *in that one `Grid*`**, not
   across `Grid`s — splitting what should be one aligned table into N per-row `Grid`s
   is the most likely root cause of "each row's label/value column is a different
   width."
3. **`Star` columns squeeze, they do not reflow.** Don't reach for `Star` columns
   expecting cards/swatches to wrap to fewer per row on a narrow window — use
   `VariableSizedWrapGrid` for that instead.
4. **`HorizontalAlignment::Stretch` only fills the space the immediate parent already
   gave the element.** It cannot make a child in one `StackPanel` match the width of an
   unrelated element in a different container.
5. **`Property::Inherited` (the default for width/alignment/margin/etc.) means "native
   WinUI default for that control type,"** not "same as siblings" — different control
   types (e.g. `TextBlock` vs. `Button`) can have different native default alignments,
   so mixing control types in one row without explicit alignment can look inconsistent
   even before any width mismatch.
6. **`VariableSizedWrapGrid.item_width`/`item_height` is one size for the whole panel**
   — you cannot give some cells a different width than others in the same panel.
7. **`ResourceOverrides` cannot help with non-`Button` sizing/style problems** — it's
   wired to exactly one control (`PropertyId::ButtonResources`), so don't reach for it
   to fix a `ContentDialog`, `Grid`, or panel's native style constants (e.g. a dialog's
   default max width) — there is no builder for that on any type other than `Button`.
8. **No keyboard events to react to per-keystroke layout/validation feedback on a
   control** — if a design calls for "resize/react as the user types a specific key,"
   that has to be driven off `on_text_changed`/`on_value_changed` (fires after the
   value settles) rather than a raw key event, because none exists.
9. **Grid row/column placement panics on negative values** (`grid_row`/`grid_column`
   assert `>= 0`) and unset placement defaults to `(0, 0)` with span `1` — an
   accidentally-omitted `.grid_row(i)` in a loop silently stacks every item in row 0
   rather than erroring.
