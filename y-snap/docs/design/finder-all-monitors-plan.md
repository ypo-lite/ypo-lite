# Mouse finder on every monitor

Idea (2026-11-15): spotlight, crosshair and monitor draw one overlay, on the
pointer's monitor only. Let the user choose to have the indicator on every
monitor, knowing it costs memory (one overlay surface, a full-monitor pixel
buffer, per extra monitor while the animation plays).

## Built

* `all-monitors="true|false"` in the look (`<finder>` and `<style>`, cascading
  like the rest), default `false`. Read, written, carried by the Test message
  (`ipc::pack_look` bit 48) and a switch in `y-snap-config` (shown for the
  shapes that honour it).
* `Shape::spreads()` says which shapes honour it: **`monitor` only**. With
  the option on the pulsing border is dropped (the map already highlights the
  pointer's monitor) and every monitor gets a *docked* surface: just the map
  panel, at the position the map has on a full-monitor surface, at that
  monitor's scale (`finder.rs`, `docked`, `Extra`; `finder_overlay.rs`,
  `map_panel_size`). The pointer's monitor uses the same layout; the extras
  are the other monitors and are rebuilt when the pointer changes monitor.
  With the option off nothing changed: border and map on a monitor-sized
  surface.

## Not built: spotlight and crosshair

On a monitor without the pointer, drawing the same thing is meaningless (a
spotlight on nothing). They need a smooth "look that way" cue instead. Open
questions, to settle before building:

* What is drawn: a soft glow along the screen edge facing the pointer, an
  arrow, or a dim with a lit edge? Spotlight could dim the other monitors
  lightly and light the edge nearest the pointer.
* Direction: from the monitor's centre towards the pointer, snapped to the
  nearest edge, or an exact angle (needs care with monitors arranged
  diagonally)?
* Animation: fade in with the main one, pulse towards the pointer's side, or
  follow the pointer live (the pointer is read every frame, so the direction
  can change while it moves)?
* Memory: a glow on an edge only needs a strip, not a full-monitor surface.
  Worth a smaller surface for the extras once the cue is chosen.

Once settled, `spreads()` grows those shapes and `Run::frame` draws the cue
with a separate function instead of the shared frame.
