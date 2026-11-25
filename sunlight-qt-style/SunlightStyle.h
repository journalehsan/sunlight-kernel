#pragma once

#include <QProxyStyle>

class SunlightStyle final : public QProxyStyle {
public:
    SunlightStyle();
    void drawControl(ControlElement, const QStyleOption *, QPainter *,
                     const QWidget * = nullptr) const override;
    void drawPrimitive(PrimitiveElement, const QStyleOption *, QPainter *,
                       const QWidget * = nullptr) const override;
    int pixelMetric(PixelMetric, const QStyleOption * = nullptr,
                    const QWidget * = nullptr) const override;
    QSize sizeFromContents(ContentsType, const QStyleOption *, const QSize &,
                           const QWidget * = nullptr) const override;
};
