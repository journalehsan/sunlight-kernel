#include "ThemeTokens.h"

#include <QApplication>
#include <QCheckBox>
#include <QComboBox>
#include <QDebug>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QImage>
#include <QLabel>
#include <QLineEdit>
#include <QMenu>
#include <QPushButton>
#include <QRadioButton>
#include <QStyleFactory>
#include <QTextEdit>
#include <QVBoxLayout>

namespace {
constexpr auto sunlightKey = "Sunlight";

QStyle *loadSunlight()
{
    auto *style = QStyleFactory::create(QString::fromLatin1(sunlightKey));
    if (!style)
        qCritical() << "Sunlight plugin unavailable. Paths:" << QCoreApplication::libraryPaths()
                    << "Styles:" << QStyleFactory::keys()
                    << "Rerun with QT_DEBUG_PLUGINS=1 for loader diagnostics.";
    return style;
}

int smoke()
{
    if (auto *style = loadSunlight())
        QApplication::setStyle(style);
    else
        return 1;

    QWidget sample;
    auto *layout = new QVBoxLayout(&sample);
    auto *button = new QPushButton("&Send");
    button->setDefault(true);
    layout->addWidget(button);
    layout->addWidget(new QLineEdit("http://example.com/"));
    auto *choice = new QCheckBox("Checked");
    choice->setChecked(true);
    layout->addWidget(choice);
    sample.resize(320, 140);
    sample.ensurePolished();
    for (const bool dark : {false, true}) {
        QApplication::setPalette(ThemeTokens::palette(dark));
        QImage image(sample.size(), QImage::Format_ARGB32_Premultiplied);
        if (image.isNull())
            return 2;
        image.fill(Qt::transparent);
        sample.render(&image);
        const auto background = image.pixelColor(0, 0);
        if (background.alpha() != 255 || image.pixelColor(button->geometry().topLeft() + QPoint(4, 4)) == background) {
            qCritical() << "Sample render did not paint the button and background.";
            return 3;
        }
    }
    return 0;
}
}

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    QCoreApplication::addLibraryPath(QStringLiteral(SUNLIGHT_BUILD_PLUGIN_ROOT));
    if (app.arguments().contains(QStringLiteral("--smoke")))
        return smoke();

    auto *style = loadSunlight();
    if (!style)
        return 1;
    QApplication::setStyle(style);
    QApplication::setPalette(ThemeTokens::palette(false));

    QWidget window;
    window.setWindowTitle("Sunlight API Lab — Qt style preview");
    window.resize(1140, 760);
    auto *gallery = new QVBoxLayout(&window);
    gallery->setContentsMargins(12, 12, 12, 12);
    gallery->setSpacing(10);

    auto *controls = new QHBoxLayout;
    auto *styles = new QComboBox;
    styles->addItems({"Sunlight", "Fusion"});
    auto *palettes = new QComboBox;
    palettes->addItems({"Light", "Dark"});
    controls->addWidget(new QLabel("Style:"));
    controls->addWidget(styles);
    controls->addWidget(new QLabel("Palette:"));
    controls->addWidget(palettes);
    controls->addStretch();
    gallery->addLayout(controls);

    auto *request = new QHBoxLayout;
    auto *method = new QComboBox;
    method->addItems({"GET", "POST", "PUT", "PATCH", "DELETE"});
    request->addWidget(method);
    auto *url = new QLineEdit("http://example.com/");
    url->setTextMargins(ThemeTokens::previewFieldMargin, 0, ThemeTokens::previewFieldMargin, 0);
    url->setPlaceholderText("Enter URL (http:// or https://)");
    request->addWidget(url, 1);
    auto *send = new QPushButton("&Send");
    send->setDefault(true);
    send->setMinimumWidth(88);
    request->addWidget(send);
    gallery->addLayout(request);

    auto *actions = new QHBoxLayout;
    actions->addWidget(new QPushButton("Clear &Req"));
    auto *disabled = new QPushButton("Clear Resp");
    disabled->setEnabled(false);
    actions->addWidget(disabled);
    auto *copy = new QPushButton("Copy &All");
    copy->setCheckable(true);
    copy->setChecked(true);
    actions->addWidget(copy);
    auto *menuButton = new QPushButton("&Edit menu");
    auto *menu = new QMenu(menuButton);
    menu->addAction("Cut\tCtrl+X")->setEnabled(false);
    menu->addAction("Copy\tCtrl+C");
    menu->addAction("Paste\tCtrl+V");
    menu->addSeparator();
    auto *wordWrap = menu->addAction("Word wrap");
    wordWrap->setCheckable(true);
    wordWrap->setChecked(true);
    auto *more = menu->addMenu("More commands");
    more->addAction("Select All\tCtrl+A");
    menuButton->setMenu(menu);
    actions->addWidget(menuButton);
    actions->addStretch();
    gallery->addLayout(actions);

    auto *choices = new QGroupBox("Choice controls");
    auto *choiceLayout = new QHBoxLayout(choices);
    auto *checks = new QVBoxLayout;
    checks->addWidget(new QCheckBox("Show &headers"));
    auto *checked = new QCheckBox("Include request &body");
    checked->setChecked(true);
    checks->addWidget(checked);
    auto *disabledCheck = new QCheckBox("A longer disabled checkbox label");
    disabledCheck->setChecked(true);
    disabledCheck->setEnabled(false);
    checks->addWidget(disabledCheck);
    choiceLayout->addLayout(checks);
    auto *radios = new QVBoxLayout;
    auto *params = new QRadioButton("&Params");
    params->setChecked(true);
    radios->addWidget(params);
    radios->addWidget(new QRadioButton("Bod&y"));
    auto *offline = new QRadioButton("Auth unavailable");
    offline->setEnabled(false);
    radios->addWidget(offline);
    choiceLayout->addLayout(radios);
    gallery->addWidget(choices);

    auto *username = new QLineEdit;
    username->setTextMargins(ThemeTokens::previewFieldMargin, 0, ThemeTokens::previewFieldMargin, 0);
    username->setPlaceholderText("Username");
    username->setEnabled(false);
    gallery->addWidget(username);

    auto *console = new QGroupBox("Console — Fusion fallback");
    auto *consoleLayout = new QVBoxLayout(console);
    auto *editor = new QTextEdit;
    editor->setPlainText("Request body\n\nThis preview does not send network requests.");
    consoleLayout->addWidget(editor);
    gallery->addWidget(console, 1);

    QObject::connect(palettes, &QComboBox::currentIndexChanged, &window, [=](int index) {
        QApplication::setPalette(ThemeTokens::palette(index == 1));
    });
    QObject::connect(styles, &QComboBox::currentIndexChanged, &window, [=](int index) {
        auto *next = index == 0 ? loadSunlight() : QStyleFactory::create("Fusion");
        if (next) {
            QApplication::setStyle(next);
            QApplication::setPalette(ThemeTokens::palette(palettes->currentIndex() == 1));
        } else {
            styles->setCurrentIndex(index == 0 ? 1 : 0);
        }
    });
    QObject::connect(send, &QPushButton::clicked, editor, [=] {
        editor->append("Preview: " + method->currentText() + " " + url->text());
    });
    window.show();
    return app.exec();
}
