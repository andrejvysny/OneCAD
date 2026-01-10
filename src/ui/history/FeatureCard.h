/**
 * @file FeatureCard.h
 * @brief Widget representing a single operation in the history timeline.
 */
#ifndef ONECAD_UI_HISTORY_FEATURECARD_H
#define ONECAD_UI_HISTORY_FEATURECARD_H

#include <QWidget>
#include <QString>

class QLabel;
class QToolButton;

namespace onecad::ui {

class FeatureCard : public QWidget {
    Q_OBJECT

public:
    explicit FeatureCard(QWidget* parent = nullptr);

    void setName(const QString& name);
    void setDetails(const QString& details);
    // iconName should be resource path, e.g. ":/icons/ic_extrude.svg"
    void setIconPath(const QString& path);
    void setFailed(bool failed, const QString& reason = {});
    void setSuppressed(bool suppressed);
    void setSelected(bool selected);
    void updateTheme();

signals:
    void menuRequested();
    void suppressToggled();

protected:
    void enterEvent(QEnterEvent* event) override;
    void leaveEvent(QEvent* event) override;

private:
    void updateStyle();
    void updateText();

    QLabel* iconLabel_ = nullptr;
    QLabel* textLabel_ = nullptr;
    QToolButton* statusButton_ = nullptr; 
    QToolButton* overflowButton_ = nullptr;

    QString name_;
    QString details_;
    QString iconPath_;
    bool failed_ = false;
    bool suppressed_ = false;
    bool selected_ = false;
    bool hovered_ = false;
    QString failureReason_;
};

} // namespace onecad::ui

#endif // ONECAD_UI_HISTORY_FEATURECARD_H
