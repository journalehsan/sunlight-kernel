#include "SunlightStyle.h"
#include "ThemeTokens.h"

#include <QPainter>
#include <QMenu>
#include <QStyleFactory>
#include <QStyleOption>

#include <algorithm>

namespace {
using namespace ThemeTokens;

void box(QPainter *painter, const QRect &rect, const QColor &fill,
         const QColor &outline = {}, int radius = squareRadius, int width = borderWidth)
{
    if (rect.width() < width || rect.height() < width)
        return;
    if (!outline.isValid() && !radius) {
        painter->fillRect(rect, fill);
        return;
    }
    painter->save();
    painter->setRenderHint(QPainter::Antialiasing, radius != 0);
    painter->setBrush(fill.isValid() ? QBrush(fill) : QBrush(Qt::NoBrush));
    painter->setPen(outline.isValid() ? QPen(outline, width) : QPen(Qt::NoPen));
    const qreal inset = outline.isValid() ? width / 2.0 : 0.0;
    painter->drawRoundedRect(QRectF(rect).adjusted(inset, inset, -inset, -inset),
                             radius, radius);
    painter->restore();
}
}

SunlightStyle::SunlightStyle() : QProxyStyle(QStyleFactory::create(QStringLiteral("Fusion")))
{
    setObjectName(QStringLiteral("Sunlight"));
}

void SunlightStyle::drawControl(ControlElement element, const QStyleOption *option,
                                QPainter *painter, const QWidget *widget) const
{
    const auto theme = colors(option->palette);
    const auto state = option->state;
    const bool enabled = state & State_Enabled;
    if (element == CE_PushButton) {
        const auto *button = qstyleoption_cast<const QStyleOptionButton *>(option);
        if (button && !(button->features & QStyleOptionButton::Flat)) {
            const bool primary = button->features & QStyleOptionButton::DefaultButton;
            const bool held = state & (State_Sunken | State_On);
            const bool hovered = state & State_MouseOver;
            const QColor fill = !enabled ? theme.alternate
                : primary ? (held ? theme.pressedAccent : hovered ? theme.accentHover : theme.accent)
                          : (held ? theme.border : hovered ? theme.alternate : theme.panel);
            box(painter, button->rect, fill, primary ? QColor() : theme.border);
            if (primary && enabled && hovered && !held && button->rect.height() >= primaryHoverLine)
                painter->fillRect(QRect(button->rect.left(),
                                        button->rect.bottom() - primaryHoverLine + 1,
                                        button->rect.width(), primaryHoverLine), theme.hoverLine);
            if (enabled && (state & State_HasFocus))
                box(painter, button->rect.adjusted(buttonFocusInset, buttonFocusInset,
                                                   -buttonFocusInset, -buttonFocusInset), {},
                    primary ? theme.window : theme.focus, squareRadius, focusWidth);

            auto label = *button;
            label.rect.adjust(buttonPadding, borderWidth, -buttonPadding, -borderWidth);
            label.palette.setColor(QPalette::ButtonText,
                                   !enabled ? theme.dim : primary ? theme.window : theme.text);
            QProxyStyle::drawControl(CE_PushButtonLabel, &label, painter, widget);
            return;
        }
    }
    if (element == CE_MenuItem) {
        if (const auto *item = qstyleoption_cast<const QStyleOptionMenuItem *>(option)) {
            if (item->menuItemType == QStyleOptionMenuItem::Separator && item->text.isEmpty()) {
                const QRect line = item->rect.adjusted(2 * menuInset, 0, -2 * menuInset, 0);
                painter->fillRect(line.x(), line.center().y(), line.width(), borderWidth, theme.border);
                return;
            }
            if (item->menuItemType == QStyleOptionMenuItem::Normal ||
                item->menuItemType == QStyleOptionMenuItem::DefaultItem ||
                item->menuItemType == QStyleOptionMenuItem::SubMenu) {
                if (enabled && (state & State_Selected))
                    box(painter, item->rect, theme.selection, theme.focus);
                auto label = *item;
                label.state.setFlag(State_Selected, false);
                for (const auto role : {QPalette::Text, QPalette::WindowText, QPalette::ButtonText})
                    label.palette.setColor(role, enabled ? theme.text : theme.dim);
                QProxyStyle::drawControl(element, &label, painter, widget);
                return;
            }
        }
    }
    QProxyStyle::drawControl(element, option, painter, widget);
}

void SunlightStyle::drawPrimitive(PrimitiveElement element, const QStyleOption *option,
                                  QPainter *painter, const QWidget *widget) const
{
    const auto theme = colors(option->palette);
    const auto state = option->state;
    const auto rect = option->rect;
    const bool enabled = state & State_Enabled;
    const bool focused = enabled && (state & State_HasFocus);
    switch (element) {
    case PE_PanelLineEdit: {
        const auto *frame = qstyleoption_cast<const QStyleOptionFrame *>(option);
        const QColor fill = enabled ? theme.panel : theme.alternate;
        if (!frame || frame->lineWidth > 0)
            box(painter, rect, fill, focused ? theme.focus : theme.border, squareRadius,
                focused ? focusWidth : borderWidth);
        else if (fill.isValid())
            painter->fillRect(rect, fill);
        return;
    }
    case PE_PanelMenu:
        if (widget && !qobject_cast<const QMenu *>(widget))
            break;
        box(painter, rect, theme.panel, theme.border, menuRadius);
        return;
    case PE_FrameMenu:
        if (qobject_cast<const QMenu *>(widget))
            return;
        break;
    case PE_IndicatorCheckBox:
    case PE_IndicatorRadioButton: {
        const bool radio = element == PE_IndicatorRadioButton;
        const bool active = enabled && (state & (State_MouseOver | State_Sunken | State_HasFocus));
        const QColor fill = !enabled ? theme.alternate : state & State_Sunken ? theme.border
                           : active ? theme.alternate : theme.panel;
        const QColor mark = enabled ? theme.accent : theme.dim;
        box(painter, rect, fill, active ? theme.focus : theme.border,
            radio ? rect.height() / 2 : squareRadius, focused ? focusWidth : borderWidth);
        if (radio && (state & State_On)) {
            box(painter, rect.adjusted(radioDotInset, radioDotInset, -radioDotInset, -radioDotInset),
                mark, {}, markInset);
        } else if (!radio && (state & (State_On | State_NoChange))) {
            painter->save();
            painter->setClipRect(rect, Qt::IntersectClip);
            painter->fillRect(rect.x() + markInset, rect.y() + indicatorSize / 2,
                              markLength, markWidth, mark);
            if (state & State_On)
                painter->fillRect(rect.x() + indicatorSize / 2, rect.y() + markInset,
                                  markWidth, markLength, mark);
            painter->restore();
        }
        return;
    }
    default:
        break;
    }
    QProxyStyle::drawPrimitive(element, option, painter, widget);
}

int SunlightStyle::pixelMetric(PixelMetric metric, const QStyleOption *option,
                               const QWidget *widget) const
{
    switch (metric) {
    case PM_IndicatorWidth:
    case PM_IndicatorHeight:
    case PM_ExclusiveIndicatorWidth:
    case PM_ExclusiveIndicatorHeight:
        return indicatorSize;
    case PM_CheckBoxLabelSpacing:
    case PM_RadioButtonLabelSpacing:
        return indicatorGap;
    case PM_MenuHMargin:
    case PM_MenuVMargin:
        return menuInset - borderWidth;
    case PM_MenuPanelWidth:
        return borderWidth;
    default:
        break;
    }
    return QProxyStyle::pixelMetric(metric, option, widget);
}

QSize SunlightStyle::sizeFromContents(ContentsType type, const QStyleOption *option,
                                     const QSize &contents, const QWidget *widget) const
{
    QSize size = QProxyStyle::sizeFromContents(type, option, contents, widget);
    switch (type) {
    case CT_PushButton:
        size = size.expandedTo(contents + QSize(2 * buttonPadding, 2 * borderWidth));
        size.setHeight(std::max(size.height(), controlHeight));
        break;
    case CT_LineEdit:
        size.setHeight(std::max(size.height(), controlHeight));
        break;
    case CT_CheckBox:
    case CT_RadioButton:
        size.setHeight(std::max(size.height(), choiceHeight));
        break;
    case CT_MenuItem:
        if (const auto *item = qstyleoption_cast<const QStyleOptionMenuItem *>(option)) {
            if (item->menuItemType == QStyleOptionMenuItem::Separator && item->text.isEmpty())
                size.setHeight(separatorHeight);
            else
                size.setHeight(std::max(size.height(), menuHeight));
        }
        break;
    default:
        break;
    }
    return size;
}
