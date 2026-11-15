# Mouse finder on every monitor

Idea (2026-11-15): spotlight, crosshair and monitor draw one overlay, on the
pointer's monitor only. Let the user choose to have the indicator on every
monitor, knowing it costs memory (one overlay surface per extra monitor while
the animation plays).

## Built

* `all-monitors="true|false"` in the look (`<finder>` and `<style>`, cascading
  like the rest), default `false`. Read, written, carried by the Test message
  (`ipc::pack_look` bit 48), a switch in `y-snap-config` (shown for the shapes
  that honour it, `Shape::spreads`) and tried live with
  `cargo run -p y-snap --example finder_lab`.
* `Spread` in `finder.rs` says what the other monitors show. The pointer's
  monitor is unchanged except for `monitor`.
* **`monitor`** (`Spread::Map`): the pulsing border is dropped (the map
  already highlights the pointer's monitor) and every monitor gets a *docked*
  surface: just the map panel, where the map has always been, at that
  monitor's scale (`finder::overlay::map_panel_size`).
* **`spotlight`** (`Spread::Glow`): the pointer's monitor keeps its spotlight;
  each other monitor gets a *strip* along the side that faces the pointer
  (`glow_strip`, a monitor wide by `glow_depth` ≈ 0.6 × radius, about 1 MB on
  a 2560×1440 screen instead of 14.7 MB) with a soft light in the style's
  colour bleeding in from the edge, centred on the border point nearest the
  pointer (`facing`). It is brighter and deeper the closer the pointer is,
  pulses gently, and follows the pointer every frame. The side is kept until
  another one is clearly better (48 px + 10 %), so a pointer near the
  diagonal does not flicker; when it does change the strip is rebuilt on the
  new side. The other monitors are not dimmed: the user may be working there,
  and a dim would need a full-size surface.

  To judge live: glow size and brightness (`size`, `color`), whether the
  pulse is wanted, the hysteresis, and how it reads on diagonal layouts.

## Not built: crosshair

Decided (2026-11-15): **an arrow** on each other monitor, pointing at the
pointer. Continuing the lines was rejected: with the pointer at a monitor's
edge the lines are hard to see, lines on every monitor would not say where
the pointer is, and on some layouts they would not reach another monitor at
all.

Not designed yet:

* Where the arrow sits: at the monitor's centre, or near the edge facing the
  pointer (reusing `facing`, which already gives the side and the closest
  point)?
* Its direction: an exact angle from the monitor's centre (or the edge point)
  to the pointer, so it is right on diagonal layouts too.
* Its size and animation: grow in with the main crosshair, a slight pulse
  along its direction; drawn on a small surface (a square a few radii wide)
  rather than the whole monitor.

## Later: declinations as styles

More cues (a different glow, an arrow on a spotlight...) should be new
*styles* (shapes) built from these pieces, not more options.
