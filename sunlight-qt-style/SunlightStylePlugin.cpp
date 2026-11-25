#include "SunlightStyle.h"

#include <QStylePlugin>

class SunlightStylePlugin final : public QStylePlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.qt-project.Qt.QStyleFactoryInterface" FILE "sunlight.json")

public:
    QStyle *create(const QString &key) override
    {
        return key.compare(QStringLiteral("Sunlight"), Qt::CaseInsensitive) == 0
            ? new SunlightStyle : nullptr;
    }
};

#include "SunlightStylePlugin.moc"
