# Sunlight Writer

**Status:** UI-only professional document shell (`sunlight-writer`).

## Overview

`sunlight-writer` is the first premium document-application shell for
SunlightOS. This phase only establishes the window layout and interaction model:

- application menu with a two-column `Open -> Recent Documents` panel
- ribbon-style command surface
- large central white document placeholder
- status bar and professional workspace framing

## Not In This Phase

- real canvas widget
- document editing logic
- file open/save implementation
- document model, formatting engine, or export pipeline

## Future Integration

The future canvas widget should replace the placeholder drawing inside the main
document surface area in `sunlight-writer/src/main.rs`, using the bounded
`canvas_insertion_rect()` layout region.
