#include "ThemeTokens.h"

#include <QApplication>
#include <QDebug>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QStyleFactory>
#include <QVBoxLayout>

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    QCoreApplication::addLibraryPath(QStringLiteral(SUNLIGHT_BUILD_PLUGIN_ROOT));
    auto *style = QStyleFactory::create(QStringLiteral("Sunlight"));
    if (!style) {
        qCritical() << "Sunlight style plugin could not be loaded.";
        return 1;
    }
    QApplication::setStyle(style);
    QApplication::setPalette(ThemeTokens::palette(true));

    QWidget window;
    window.setWindowTitle("Hello World — Sunlight Dark");
    auto *layout = new QVBoxLayout(&window);
    layout->setContentsMargins(24, 24, 24, 24);
    layout->setSpacing(12);

    auto *greeting = new QLabel("Hello, world!");
    auto font = greeting->font();
    font.setPixelSize(26);
    font.setBold(true);
    greeting->setFont(font);
    greeting->setTextFormat(Qt::PlainText);
    layout->addWidget(greeting);
    layout->addWidget(new QLabel("Qt Widgets · Sunlight dark"));

    auto *name = new QLineEdit;
    name->setPlaceholderText("Your name");
    name->setAccessibleName("Your name");
    name->setMaxLength(32);
    layout->addWidget(name);

    auto *button = new QPushButton("&Say hello");
    button->setDefault(true);
    layout->addWidget(button);
    const auto sayHello = [=] {
        const auto entered = name->text().trimmed();
        greeting->setText(entered.isEmpty() ? QStringLiteral("Hello, world!")
                                            : QStringLiteral("Hello, %1!").arg(entered));
        greeting->setWordWrap(true);
    };
    QObject::connect(button, &QPushButton::clicked, &window, sayHello);
    QObject::connect(name, &QLineEdit::returnPressed, &window, sayHello);

    window.resize(420, 208);
    window.show();
    return app.exec();
}
