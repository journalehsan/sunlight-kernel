#pragma once

#include <QColor>
#include <QPalette>

namespace ThemeTokens {

inline constexpr int borderWidth = 1;
inline constexpr int focusWidth = 2;
inline constexpr int buttonFocusInset = 2;
inline constexpr int squareRadius = 0;
inline constexpr int menuRadius = 7;
inline constexpr int menuInset = 4;
inline constexpr int buttonPadding = 8;
inline constexpr int fieldPadding = 6;
inline constexpr int previewFieldMargin = fieldPadding - 3;
inline constexpr int indicatorSize = 14;
inline constexpr int indicatorGap = 6;
inline constexpr int markInset = 3;
inline constexpr int markLength = 8;
inline constexpr int markWidth = 2;
inline constexpr int radioDotInset = 4;
inline constexpr int controlHeight = 28;
inline constexpr int choiceHeight = 18;
inline constexpr int menuHeight = 24;
inline constexpr int separatorHeight = 9;
inline constexpr int primaryHoverLine = 2;

struct Colors {
    QColor window, panel, alternate, text, dim, accent, accentHover, border;
    QColor pressedAccent, hoverLine, selection, focus;
};

inline Colors colors(bool dark)
{
    return dark
        ? Colors{0x121214, 0x1c1c1f, 0x222226, 0xf0f0f0, 0x888899,
                 0xffa500, 0xffbf40, 0x353540, 0xd78b00, 0xffce6e, 0x362b1a, 0xffa500}
        : Colors{0xf2f2f5, 0xffffff, 0xebebef, 0x111111, 0x666677,
                 0xe68a00, 0xffa500, 0xccccd6, 0xc27300, 0xffba3c, 0xf5ead9, 0x8a4c00};
}

inline Colors colors(const QPalette &palette)
{
    return colors(palette.color(QPalette::Active, QPalette::Window).lightness() < 128);
}

inline QPalette palette(bool dark)
{
    const auto theme = colors(dark);
    QPalette result;
    result.setColor(QPalette::Window, theme.window);
    result.setColor(QPalette::Base, theme.panel);
    result.setColor(QPalette::AlternateBase, theme.alternate);
    result.setColor(QPalette::Button, theme.panel);
    result.setColor(QPalette::Highlight, theme.selection);
    result.setColor(QPalette::HighlightedText, theme.text);
    result.setColor(QPalette::Light, theme.alternate);
    result.setColor(QPalette::Midlight, theme.border);
    result.setColor(QPalette::Mid, theme.border);
    result.setColor(QPalette::Dark, theme.dim);
    result.setColor(QPalette::Shadow, theme.border);
    result.setColor(QPalette::ToolTipBase, theme.panel);
    result.setColor(QPalette::ToolTipText, theme.text);
    result.setColor(QPalette::Link, theme.focus);
    result.setColor(QPalette::PlaceholderText, theme.dim);
    for (const auto role : {QPalette::Text, QPalette::WindowText, QPalette::ButtonText}) {
        result.setColor(role, theme.text);
        result.setColor(QPalette::Disabled, role, theme.dim);
    }
    result.setColor(QPalette::Disabled, QPalette::Base, theme.alternate);
    return result;
}

}
