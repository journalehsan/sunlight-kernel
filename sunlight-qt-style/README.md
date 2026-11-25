# Sunlight Qt Widgets style

A standalone C++17 Qt Widgets plugin and Linux preview. The dynamic plugin
creates a fresh Fusion-backed proxy for each request for the `Sunlight` key.
The preview depends on the plugin at build time so it is available to Qt's
loader, and links only to Qt Widgets.

```text
sunlight-qt-style/
├── CMakeLists.txt
├── HelloWorld.cpp             # Small Sunlight dark example
├── README.md
├── Preview.cpp                # Gallery and CTest render check
├── SunlightStyle.cpp
├── SunlightStyle.h
├── SunlightStylePlugin.cpp
├── ThemeTokens.h
└── sunlight.json
```

## Build and run on Linux

Install a C++17 compiler, CMake 3.16 or newer and Qt 6 Widgets development
files with the desktop and offscreen platform plugins. **Qt 6.2** is the
minimum supported baseline, chosen as the first Qt 6 long-term support
release. The code uses public Widgets APIs available at that baseline.

From the repository root:

```sh
cmake -S sunlight-qt-style -B target/sunlight-qt-style -DCMAKE_BUILD_TYPE=Release
cmake --build target/sunlight-qt-style --parallel
QT_SCALE_FACTOR=1 ./target/sunlight-qt-style/sunlight-preview
ctest --test-dir target/sunlight-qt-style --output-on-failure
```

The preview registers `target/sunlight-qt-style/plugins` with
`QCoreApplication::addLibraryPath()` before calling
`QStyleFactory::create("Sunlight")`. The module is in that root's `styles/`
subdirectory. Use the root, not the `styles/` directory, when registering it.
If discovery fails, check the searched paths, metadata and loader errors with:

```sh
QT_DEBUG_PLUGINS=1 ./target/sunlight-qt-style/sunlight-preview
```

No global `QT_PLUGIN_PATH` is needed. The CTest uses the offscreen platform,
clears `QT_PLUGIN_PATH`, requires the factory to load the plugin, and renders
a sample in both palettes. It checks for painted output; this does not
establish visual fidelity.

Run the standalone dark example with:

```sh
./target/sunlight-qt-style/sunlight-hello
```

Type a name and press Enter or choose **Say hello** to update the greeting.

## Reference and comparison

The source of truth is the in-tree `sunlight-ui/theme.rs` and
`sunlight-ui/widgets/{button,text_input,checkbox,text_context_menu}.rs`.
The API Lab (`sunlight-api-lab/src/main.rs`) supplies the 1140 × 760 logical
window size, 28-pixel action and field geometry, and the request and console
examples. The native widget gallery shows clocks, numbers and a map; API Lab
is a closer comparison for the controls in scope. Run the preview at 1× on
an unscaled desktop and compare to API Lab at 1×. Use the same palette;
tab through the controls, hold Space on a button, and open Edit menu to
compare the keyboard highlight. Qt uses logical coordinates throughout.

| Qt control | Source-derived details | Approximation or Qt behavior |
| --- | --- | --- |
| Push button | Square outline; primary orange fill and hover underline; secondary and disabled fills; 28-pixel minimum from API Lab | Default button maps to primary, checked maps to pressed; horizontal padding and inset focus ring are preview choices. Qt draws text, icons and mnemonics. Flat buttons stay on Fusion. |
| Line edit | Square panel and inset border, active outline, 28-pixel minimum; preview margins target the reference's 6-pixel text inset | Stronger focus outline and disabled fill are adaptations; exact text inset depends on Fusion frame metrics. Qt owns text, selection and caret. |
| Checkbox | 14-pixel square, 6-pixel label gap, orange plus mark | Qt centers the indicator and lays out the label; mixed, disabled, hover and keyboard states extend the reference. |
| Radio button | Shares checkbox semantic colors and dimensions | Provisional circular version; no generic radio widget was found in the reference. |
| Menu | Panel radius 7, inset 4, 24-pixel row minimum, 9-pixel unlabeled separators | Highlight uses the toolkit's selection color with a focus outline. Qt handles menu text, shortcut and icon columns, checkable actions and submenu arrows; widths grow with content. |

`ThemeTokens.h` centralizes the source palette and geometry plus the
provisional choices. The preview reuses API Lab labels including Send,
Clear Req, Clear Resp and Copy All, plus its example URL. It rearranges the
request strip and console into a widget gallery, adds state examples and
omits the native app's networking and full layout. Qt uses its normal font
and rendering backend. The multiline editor, combo boxes, group boxes and
other unstyled controls use Fusion. The Fusion comparison uses the same
preview palette so that geometry and painting differences are easier to see.

## Install and rebuild for SunApp

The configurable CMake cache variable `SUNLIGHT_QT_PLUGIN_INSTALL_ROOT`
defaults to `plugins` relative to the install prefix. For a Linux staging
check with that default:

```sh
cmake --install target/sunlight-qt-style --prefix "$PWD/target/sunlight-qt-style/stage"
```

This produces `stage/plugins/styles/libsunlightstyle.so` under the build
directory. The install target contains the plugin; the preview runs from the
build tree.

For SunApp, configure the **same source** in a new build directory with the
actual Sunlight SDK toolchain and target Qt package. If cross-compiling, use
the SDK's required host Qt build tools and disable host CTest execution with
`-DBUILD_TESTING=OFF` where appropriate. Set
`SUNLIGHT_QT_PLUGIN_INSTALL_ROOT` to the bundle's chosen plugin root relative
to its staging prefix. Rebuild against the Qt used by the target app so the
plugin matches its ABI and architecture.

Configure the relocated app's Qt plugin root through the bundle's supported
mechanism, e.g. the app calls `QCoreApplication::addLibraryPath()` with its
resolved bundle plugin root, or the bundle provides a `qt.conf` beside the
executable if supported. For a layout with a `plugins` directory next to
the executable, the Qt configuration would be:

```ini
[Paths]
Prefix=.
Plugins=plugins
```

This is an example layout only. The SDK configuration, bundle paths and
runtime loader must be verified with the actual SunApp tooling. The SDK
rebuild and a native side-by-side visual comparison are not covered by
the Linux smoke test.
