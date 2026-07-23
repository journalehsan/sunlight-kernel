# Rappid Rabbit chrome polish audit

## URL bar baseline

Rappid Rabbit uses the shared bounded `sunlight_ui::widgets::TextInput` for
its URL field.  It is constructed with the regular UI font and a URL
placeholder, then its rectangle is assigned during `draw_top_bar`.

The shared widget already provides UTF-8-safe single-line editing, caret and
selection painting, drag selection, horizontal cursor-relative text visibility,
`Ctrl+A/C/X/V`, clipboard access, a text context menu, and an I-beam request
over its editable rectangle.  Rabbit supplies Enter-to-navigate: an active URL
field receiving Enter queues the normal navigation flow.  Ctrl+L/F6 clears and
focuses that same field.

The request is queued and then fetched synchronously from Rabbit's next local
tick, so there is no concurrently delivered network callback that can replace
the user's edits.  The field is normalized before navigation.  Before this
patch, a redirect's final URL was retained by the network/render state but was
not copied back into the field.

Visually, the browser passed the generic field rectangle directly to
`TextInput::draw`.  That gave the URL bar a square generic panel, uniform
padding, and only active-vs-idle border treatment.  It had no browser-specific
site indicator, action area, hover treatment, or rounded primary-control
surface.

## Context-menu baseline

There is no separate browser-page context-menu model.  The existing browser
context menu is the shared text-editing menu opened by a right click in the URL
field.  `TextInput` detects right button 1, builds `TextMenuState` from the
field/clipboard state, and opens `TextContextMenu` inside the application
window.  Its editable menu contains Cut, Copy, Paste, Delete, and Select All
(with disabled rows where state does not permit an action); it measures each
24px row plus separators and vertical padding rather than using a fixed
one-row height.  Hit testing uses the same row rectangles used for painting.

The menu is an in-window overlay, not a child compositor surface or popup
window.  It is clamped to the active client bounds and has no ancestor clipping
operation.  Rabbit, however, called `url_input.draw()` while drawing its top
bar, then painted Source/DocumentCanvas content and the developer-tools panel
afterwards.  The menu therefore existed and had multiple laid-out rows, but
later browser painting obscured rows below the chrome boundary.  Event routing
was already safe because Rabbit dispatches events to `url_input` first, so an
open menu consumes its press/click interaction before page controls.

## Classification

- **Paint-order overlay defect:** the text menu was painted below later page
  and developer-tool surfaces.
- **Purely visual polish issue:** the generic field did not express browser
  control hierarchy, focus/hover state, embedded regions, or rounded Sunlight
  chrome.
- **Not present:** menu-model population, measurement, row-layout, clipping,
  hit-testing, stale-state, or browser-context-detection defects.  The menu
  model and its row geometry were already complete.

## Patch boundary

The patch keeps the shared editing and clipboard behavior intact.  It adds
separate text-content and menu-overlay draw entry points to `TextInput`, lets
Rabbit render a browser-specific URL field around that shared editing surface,
and paints the menu last.  It does not add page-level context actions, a tab
model, networking changes, or developer-tool redesign.
