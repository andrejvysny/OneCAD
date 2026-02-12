#include "MainWindow.h"
#include "../theme/ThemeManager.h"
#include "../viewport/Viewport.h"
#include "../viewport/RenderDebugPanel.h"
#include "../viewport/SnapSettingsPanel.h"
#include "../../render/Camera3D.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/commands/DeleteBodyCommand.h"
#include "../../app/commands/DeleteSketchCommand.h"
#include "../../app/commands/RemoveOperationCommand.h"
#include "../../app/commands/RenameBodyCommand.h"
#include "../../app/commands/RenameSketchCommand.h"
#include "../../app/commands/RollbackCommand.h"
#include "../../app/commands/SetOperationSuppressionCommand.h"
#include "../../app/commands/ToggleVisibilityCommand.h"
#include "../../app/document/Document.h"
#include "../../app/history/RegenerationEngine.h"
#include "../navigator/ModelNavigator.h"
#include "../toolbar/ContextToolbar.h"
#include "../sketch/ConstraintPanel.h"
#include "../sketch/ConstraintApplicability.h"
#include "../sketch/SketchModePanel.h"
#include "../../core/sketch/SketchRenderer.h"
#include "../../core/sketch/SketchTypes.h"
#include "../../app/selection/SelectionTypes.h"
#include <QMenuBar>
#include <QStatusBar>
#include <QLabel>
#include <QSlider>
#include <QApplication>
#include <QMessageBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QActionGroup>
#include <QSettings>
#include <QHBoxLayout>
#include <QWidget>
#include <QSizePolicy>
#include <QEvent>
#include <QDir>
#include <QInputDialog>
#include <QLineEdit>
#include <QFile>
#include <QTimer>
#include <QShortcut>
#include <QDebug>
#include <algorithm>
#include <unordered_set>

#include "../../io/OneCADFileIO.h"
#include "../../io/step/StepImporter.h"
#include "../../io/step/StepExporter.h"

#include "../components/SidebarToolButton.h"
#include "../start/StartOverlay.h"
#include "../history/HistoryPanel.h"
#include "../history/RegenFailureDialog.h"

namespace onecad {
namespace ui {

namespace {
// Default constraint values
constexpr double kDefaultDistanceMm = 10.0;
constexpr double kDefaultAngleDeg = 90.0;
constexpr double kDefaultRadiusMm = 10.0;

QString formatOperationDisplayName(const app::OperationRecord& op) {
    QString typeName;
    QString params;

    switch (op.type) {
        case app::OperationType::Extrude:
            typeName = "Extrude";
            if (std::holds_alternative<app::ExtrudeParams>(op.params)) {
                const auto& p = std::get<app::ExtrudeParams>(op.params);
                params = QString(" (%1mm)").arg(p.distance, 0, 'f', 1);
            }
            break;
        case app::OperationType::Revolve:
            typeName = "Revolve";
            if (std::holds_alternative<app::RevolveParams>(op.params)) {
                const auto& p = std::get<app::RevolveParams>(op.params);
                params = QString(" (%1°)").arg(p.angleDeg, 0, 'f', 0);
            }
            break;
        case app::OperationType::Fillet:
            typeName = "Fillet";
            if (std::holds_alternative<app::FilletChamferParams>(op.params)) {
                const auto& p = std::get<app::FilletChamferParams>(op.params);
                params = QString(" (R%1)").arg(p.radius, 0, 'f', 1);
            }
            break;
        case app::OperationType::Chamfer:
            typeName = "Chamfer";
            if (std::holds_alternative<app::FilletChamferParams>(op.params)) {
                const auto& p = std::get<app::FilletChamferParams>(op.params);
                params = QString(" (%1)").arg(p.radius, 0, 'f', 1);
            }
            break;
        case app::OperationType::Shell:
            typeName = "Shell";
            if (std::holds_alternative<app::ShellParams>(op.params)) {
                const auto& p = std::get<app::ShellParams>(op.params);
                params = QString(" (%1mm)").arg(p.thickness, 0, 'f', 1);
            }
            break;
        case app::OperationType::Boolean:
            typeName = "Boolean";
            if (std::holds_alternative<app::BooleanParams>(op.params)) {
                const auto& p = std::get<app::BooleanParams>(op.params);
                switch (p.operation) {
                    case app::BooleanParams::Op::Union: params = " (Union)"; break;
                    case app::BooleanParams::Op::Cut: params = " (Cut)"; break;
                    case app::BooleanParams::Op::Intersect: params = " (Intersect)"; break;
                }
            }
            break;
    }

    return typeName + params;
}
} // anonymous namespace

MainWindow::MainWindow(QWidget* parent)
    : QMainWindow(parent) {

    setWindowTitle(tr("OneCAD"));
    resize(1280, 800);
    setMinimumSize(800, 600);

    // Create document model (no Qt parent - unique_ptr manages lifetime)
    m_document = std::make_unique<app::Document>();
    m_commandProcessor = std::make_unique<app::commands::CommandProcessor>();

    applyTheme();
    setupMenuBar();
    setupViewport();
    setupToolBar();
    setupStatusBar();
    connect(&ThemeManager::instance(), &ThemeManager::themeChanged,
            this, &MainWindow::applyDofStatusStyle, Qt::UniqueConnection);

    connectDocumentSignals();

    // Connect navigator item actions
    connect(m_navigator, &ModelNavigator::deleteRequested,
            this, &MainWindow::onDeleteItem);
    connect(m_navigator, &ModelNavigator::renameRequested,
            this, &MainWindow::onRenameItem);
    connect(m_navigator, &ModelNavigator::renameCommitted, this,
            [this](const QString& itemId, const QString& newName) {
                std::string id = itemId.toStdString();
                bool isBody = m_document->getBodyShape(id) != nullptr;
                if (isBody) {
                    auto cmd = std::make_unique<app::commands::RenameBodyCommand>(
                        m_document.get(), id, newName.toStdString());
                    m_commandProcessor->execute(std::move(cmd));
                } else {
                    auto cmd = std::make_unique<app::commands::RenameSketchCommand>(
                        m_document.get(), id, newName.toStdString());
                    m_commandProcessor->execute(std::move(cmd));
                }
            });
    connect(m_navigator, &ModelNavigator::visibilityToggled,
            this, &MainWindow::onVisibilityToggled);
    connect(m_navigator, &ModelNavigator::isolateRequested,
            this, &MainWindow::onIsolateItem);

    loadSettings();

    QTimer::singleShot(0, this, &MainWindow::showStartDialog);
}

MainWindow::~MainWindow() {
    saveSettings();
}

void MainWindow::applyTheme() {
    ThemeManager::instance().applyTheme();
    applyDofStatusStyle();
}

void MainWindow::connectDocumentSignals() {
    if (!m_document || !m_navigator) {
        return;
    }

    connect(m_document.get(), &app::Document::sketchAdded,
            m_navigator, &ModelNavigator::onSketchAdded);
    connect(m_document.get(), &app::Document::sketchRemoved,
            m_navigator, &ModelNavigator::onSketchRemoved);
    connect(m_document.get(), &app::Document::sketchRenamed,
            m_navigator, &ModelNavigator::onSketchRenamed);
    connect(m_document.get(), &app::Document::bodyAdded,
            m_navigator, &ModelNavigator::onBodyAdded);
    connect(m_document.get(), &app::Document::bodyRemoved,
            m_navigator, &ModelNavigator::onBodyRemoved);
    connect(m_document.get(), &app::Document::bodyRenamed,
            m_navigator, &ModelNavigator::onBodyRenamed);
    connect(m_document.get(), &app::Document::bodyVisibilityChanged,
            m_navigator, &ModelNavigator::onBodyVisibilityChanged);
    connect(m_document.get(), &app::Document::sketchVisibilityChanged,
            m_navigator, &ModelNavigator::onSketchVisibilityChanged);

    if (m_historyPanel) {
        connect(m_document.get(), &app::Document::operationAdded,
                m_historyPanel, &HistoryPanel::onOperationAdded);
        connect(m_document.get(), &app::Document::operationRemoved,
                m_historyPanel, &HistoryPanel::onOperationRemoved);
        connect(m_document.get(), &app::Document::operationUpdated,
                m_historyPanel, &HistoryPanel::onOperationAdded);
        connect(m_document.get(), &app::Document::operationSuppressionChanged,
                m_historyPanel, &HistoryPanel::onOperationSuppressed);
        connect(m_document.get(), &app::Document::operationFailed,
                m_historyPanel, &HistoryPanel::onOperationFailed);
        connect(m_document.get(), &app::Document::operationSucceeded,
                m_historyPanel, &HistoryPanel::onOperationSucceeded);
    }
    connect(m_document.get(), &app::Document::documentCleared,
            m_navigator, [this]() { m_navigator->rebuild(m_document.get()); });
}

void MainWindow::updateDofStatus(core::sketch::Sketch* sketch) {
    if (!m_dofStatus) {
        return;
    }

    if (!sketch) {
        m_dofStatus->setText(tr("DOF: —"));
        m_dofStatus->setStyleSheet("");
        m_hasCachedDof = false;
        return;
    }

    int dof = sketch->getDegreesOfFreedom();
    bool overConstrained = sketch->isOverConstrained();
    m_dofStatus->setText(tr("DOF: %1").arg(dof));
    m_cachedDof = dof;
    m_cachedOverConstrained = overConstrained;
    m_hasCachedDof = true;

    applyDofStatusStyle();
}

void MainWindow::applyDofStatusStyle() {
    if (!m_dofStatus || !m_hasCachedDof) {
        return;
    }

    const ThemeStatusColors& status = ThemeManager::instance().currentTheme().status;
    QColor color = status.dofWarning;
    if (m_cachedOverConstrained) {
        color = status.dofError;
    } else if (m_cachedDof == 0) {
        color = status.dofOk;
    }

    m_dofStatus->setStyleSheet(QStringLiteral("color: %1;").arg(toQssColor(color)));
}

void MainWindow::setupMenuBar() {
    QMenuBar* menuBar = this->menuBar();
    
    // File menu
    QMenu* fileMenu = menuBar->addMenu(tr("&File"));
    fileMenu->addAction(tr("&New"), QKeySequence::New, this, &MainWindow::onNewDocument);
    fileMenu->addAction(tr("&Open..."), QKeySequence::Open, this, &MainWindow::onOpenDocument);
    fileMenu->addSeparator();
    fileMenu->addAction(tr("&Save"), QKeySequence::Save, this, &MainWindow::onSaveDocument);
    fileMenu->addAction(tr("Save &As..."), QKeySequence::SaveAs, this, &MainWindow::onSaveDocumentAs);
    fileMenu->addSeparator();
    fileMenu->addAction(tr("&Import STEP..."), this, &MainWindow::onImport);
    fileMenu->addAction(tr("&Export STEP..."), this, &MainWindow::onExportStep);
    fileMenu->addSeparator();
    fileMenu->addAction(tr("&Quit"), QKeySequence::Quit, qApp, &QApplication::quit);
    
    // Edit menu
    QMenu* editMenu = menuBar->addMenu(tr("&Edit"));
    m_undoAction = editMenu->addAction(tr("&Undo"), QKeySequence::Undo, this, [this]() {
        if (m_commandProcessor) {
            m_commandProcessor->undo();
        }
    });
    m_redoAction = editMenu->addAction(tr("&Redo"), QKeySequence::Redo, this, [this]() {
        if (m_commandProcessor) {
            m_commandProcessor->redo();
        }
    });
    if (m_undoAction) {
        m_undoAction->setEnabled(false);
    }
    if (m_redoAction) {
        m_redoAction->setEnabled(false);
    }
    if (m_commandProcessor) {
        if (m_undoAction) {
            connect(m_commandProcessor.get(), &app::commands::CommandProcessor::canUndoChanged,
                    m_undoAction, &QAction::setEnabled);
        }
        if (m_redoAction) {
            connect(m_commandProcessor.get(), &app::commands::CommandProcessor::canRedoChanged,
                    m_redoAction, &QAction::setEnabled);
        }
    }
    editMenu->addSeparator();
    editMenu->addAction(tr("&Delete"), QKeySequence::Delete, this, []() {});
    editMenu->addAction(tr("Select &All"), QKeySequence::SelectAll, this, []() {});
    
    // View menu
    QMenu* viewMenu = menuBar->addMenu(tr("&View"));
    viewMenu->addAction(tr("Zoom to &Fit"), QKeySequence(Qt::Key_0), this, [this]() {
        m_viewport->resetView();
    });
    viewMenu->addSeparator();
    viewMenu->addAction(tr("&Front"), QKeySequence(Qt::Key_1), this, [this]() {
        m_viewport->setFrontView();
    });
    viewMenu->addAction(tr("&Back"), QKeySequence(Qt::Key_2), this, [this]() {
        m_viewport->setBackView();
    });
    viewMenu->addAction(tr("&Left"), QKeySequence(Qt::Key_3), this, [this]() {
        m_viewport->setLeftView();
    });
    viewMenu->addAction(tr("&Right"), QKeySequence(Qt::Key_4), this, [this]() {
        m_viewport->setRightView();
    });
    viewMenu->addAction(tr("&Top"), QKeySequence(Qt::Key_5), this, [this]() {
        m_viewport->setTopView();
    });
    viewMenu->addAction(tr("Botto&m"), QKeySequence(Qt::Key_6), this, [this]() {
        m_viewport->setBottomView();
    });
    viewMenu->addAction(tr("&Isometric"), QKeySequence(Qt::Key_7), this, [this]() {
        m_viewport->setIsometricView();
    });

    viewMenu->addSeparator();

    viewMenu->addAction(tr("Toggle &Grid"), QKeySequence(Qt::Key_G), this, [this]() {
        m_viewport->toggleGrid();
    });
    viewMenu->addSeparator();

    // Theme Submenu
    QMenu* themeMenu = viewMenu->addMenu(tr("&Theme"));
    QActionGroup* themeGroup = new QActionGroup(this);
    themeGroup->setExclusive(true);

    QAction* systemAction = themeMenu->addAction(tr("&System"));
    systemAction->setCheckable(true);
    themeGroup->addAction(systemAction);
    connect(systemAction, &QAction::triggered, this, [](){
        ThemeManager::instance().setThemeMode(ThemeManager::ThemeMode::System);
    });

    themeMenu->addSeparator();

    QAction* checkedAction = nullptr;
    const auto& themes = ThemeManager::instance().availableThemes();
    for (const auto& theme : themes) {
        QAction* action = themeMenu->addAction(theme.displayName);
        action->setCheckable(true);
        action->setData(theme.id);
        themeGroup->addAction(action);
        connect(action, &QAction::triggered, this, [themeId = theme.id]() {
            ThemeManager::instance().setThemeId(themeId);
        });

        if (ThemeManager::instance().themeMode() == ThemeManager::ThemeMode::Fixed &&
            ThemeManager::instance().themeId() == theme.id) {
            checkedAction = action;
        }
    }

    if (ThemeManager::instance().themeMode() == ThemeManager::ThemeMode::System || !checkedAction) {
        systemAction->setChecked(true);
    } else {
        checkedAction->setChecked(true);
    }

    viewMenu->addSeparator();
    
    // Help menu
    QMenu* helpMenu = menuBar->addMenu(tr("&Help"));
    helpMenu->addAction(tr("&About OneCAD"), this, [this]() {
        QMessageBox::about(this, tr("About OneCAD"),
            tr("<h3>OneCAD</h3>"
               "<p>Version 0.1.0</p>"
               "<p>A beginner-friendly 3D CAD for makers.</p>"
               "<p>Built with Qt 6 + OpenCASCADE + Eigen3</p>"));
    });

    // Sketch mode keyboard shortcuts (global, work when viewport has focus)
    QAction* lineAction = new QAction(tr("Line Tool"), this);
    lineAction->setShortcut(Qt::Key_L);
    connect(lineAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateLineTool();
        }
    });
    addAction(lineAction);

    QAction* rectAction = new QAction(tr("Rectangle Tool"), this);
    rectAction->setShortcut(Qt::Key_R);
    connect(rectAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateRectangleTool();
        }
    });
    addAction(rectAction);

    QAction* circleAction = new QAction(tr("Circle Tool"), this);
    circleAction->setShortcut(Qt::Key_C);
    connect(circleAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateCircleTool();
        }
    });
    addAction(circleAction);

    QAction* arcAction = new QAction(tr("Arc Tool"), this);
    arcAction->setShortcut(Qt::Key_A);
    connect(arcAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateArcTool();
        }
    });
    addAction(arcAction);

    QAction* ellipseAction = new QAction(tr("Ellipse Tool"), this);
    ellipseAction->setShortcut(Qt::Key_E);
    connect(ellipseAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateEllipseTool();
        }
    });
    addAction(ellipseAction);

    QAction* trimAction = new QAction(tr("Trim Tool"), this);
    trimAction->setShortcut(Qt::Key_T);
    connect(trimAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateTrimTool();
        }
    });
    addAction(trimAction);

    QAction* mirrorAction = new QAction(tr("Mirror Tool"), this);
    mirrorAction->setShortcut(Qt::Key_M);
    connect(mirrorAction, &QAction::triggered, this, [this]() {
        if (m_viewport && m_viewport->isInSketchMode()) {
            m_viewport->activateMirrorTool();
        }
    });
    addAction(mirrorAction);

    QAction* escAction = new QAction(tr("Cancel/Exit"), this);
    escAction->setShortcut(Qt::Key_Escape);
    connect(escAction, &QAction::triggered, this, [this]() {
        if (m_viewport) {
            if (m_viewport->isInSketchMode()) {
                auto* tm = m_viewport->toolManager();
                if (tm && tm->hasActiveTool()) {
                    m_viewport->deactivateTool();
                } else {
                    onExitSketch();
                }
            } else if (m_viewport->isPlaneSelectionActive()) {
                m_viewport->cancelPlaneSelection();
            }
        }
    });
    addAction(escAction);
}

void MainWindow::setupToolBar() {
    if (!m_viewport) {
        return;
    }

    m_toolbar = new ContextToolbar(m_viewport);
    m_toolbar->setContext(ContextToolbar::Context::Default);
    m_toolbar->show();
    positionToolbarOverlay();

    connect(m_toolbar, &ContextToolbar::newSketchRequested,
            this, &MainWindow::onNewSketch);
    connect(m_toolbar, &ContextToolbar::extrudeRequested, this, [this]() {
        if (!m_viewport) {
            return;
        }
        const bool activated = m_viewport->activateExtrudeTool();
        if (m_toolStatus) {
            m_toolStatus->setText(activated
                ? tr("Extrude tool active")
                : tr("Select a sketch region to extrude"));
        }
        if (m_toolbar) {
            m_toolbar->setExtrudeActive(activated);
        }
    });
    connect(m_toolbar, &ContextToolbar::revolveRequested, this, [this]() {
        if (!m_viewport) {
            return;
        }
        const bool activated = m_viewport->activateRevolveTool();
        if (m_toolStatus) {
            m_toolStatus->setText(activated
                ? tr("Revolve tool active - Select axis")
                : tr("Select a sketch region or face to revolve"));
        }
        if (m_toolbar) {
            m_toolbar->setRevolveActive(activated);
        }
    });
    connect(m_toolbar, &ContextToolbar::filletRequested, this, [this]() {
        if (!m_viewport) {
            return;
        }
        const bool activated = m_viewport->activateFilletTool();
        if (m_toolStatus) {
            m_toolStatus->setText(activated
                ? tr("Fillet/Chamfer tool active - Drag to adjust")
                : tr("Select an edge to fillet or chamfer"));
        }
        if (m_toolbar) {
            m_toolbar->setFilletActive(activated);
        }
    });
    connect(m_toolbar, &ContextToolbar::shellRequested, this, [this]() {
        if (!m_viewport) {
            return;
        }
        const bool activated = m_viewport->activateShellTool();
        if (m_toolStatus) {
            m_toolStatus->setText(activated
                ? tr("Shell tool active - Shift+click faces, then drag")
                : tr("Select a body to shell"));
        }
        if (m_toolbar) {
            m_toolbar->setShellActive(activated);
        }
    });
    connect(m_toolbar, &ContextToolbar::exitSketchRequested,
            this, &MainWindow::onExitSketch);

    if (m_viewport) {
        auto activateTool = [this](void (Viewport::*slot)()) {
            (m_viewport->*slot)();
        };
        connect(m_toolbar, &ContextToolbar::lineToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateLineTool);
        });
        connect(m_toolbar, &ContextToolbar::circleToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateCircleTool);
        });
        connect(m_toolbar, &ContextToolbar::rectangleToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateRectangleTool);
        });
        connect(m_toolbar, &ContextToolbar::arcToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateArcTool);
        });
        connect(m_toolbar, &ContextToolbar::ellipseToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateEllipseTool);
        });
        connect(m_toolbar, &ContextToolbar::trimToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateTrimTool);
        });
        connect(m_toolbar, &ContextToolbar::mirrorToolActivated, this, [this, activateTool]() {
            activateTool(&Viewport::activateMirrorTool);
        });

        connect(m_viewport, &Viewport::extrudeToolActiveChanged,
                m_toolbar, &ContextToolbar::setExtrudeActive);
        connect(m_viewport, &Viewport::revolveToolActiveChanged,
                m_toolbar, &ContextToolbar::setRevolveActive);
        connect(m_viewport, &Viewport::filletToolActiveChanged,
                m_toolbar, &ContextToolbar::setFilletActive);
        connect(m_viewport, &Viewport::shellToolActiveChanged,
                m_toolbar, &ContextToolbar::setShellActive);
        connect(m_viewport, &Viewport::selectionContextChanged, this, [this](int context) {
            if (!m_toolbar || !m_viewport || m_viewport->isInSketchMode()) {
                return;
            }
            // Map int context to ContextToolbar::Context
            switch (context) {
                case 1:
                    m_toolbar->setContext(ContextToolbar::Context::Edge);
                    break;
                case 2:
                    m_toolbar->setContext(ContextToolbar::Context::Face);
                    break;
                case 3:
                    m_toolbar->setContext(ContextToolbar::Context::Body);
                    break;
                default:
                    m_toolbar->setContext(ContextToolbar::Context::Default);
                    break;
            }
        });
    }

    // Reposition toolbar when context changes (button visibility affects height)
    connect(m_toolbar, &ContextToolbar::contextChanged,
            this, &MainWindow::positionToolbarOverlay);

    // Tool activation signals - connect after m_viewport is created
    // These are connected in setupViewport() after viewport exists
}

void MainWindow::setupNavigatorOverlayButton() {
    if (!m_viewport || !m_navigator) {
        return;
    }

    m_navigatorOverlayButton = SidebarToolButton::fromSvgIcon(
        ":/icons/stack.svg", 
        tr("Toggle navigator"), 
        m_viewport
    );
    m_navigatorOverlayButton->setFixedSize(42, 42);

    connect(m_navigatorOverlayButton, &SidebarToolButton::clicked, this, [this]() {
        if (m_navigator) {
            m_navigator->setCollapsed(!m_navigator->isCollapsed());
        }
    });

    connect(m_navigator, &ModelNavigator::collapsedChanged, this, [this](bool collapsed) {
        if (m_navigatorOverlayButton) {
            positionNavigatorOverlayButton();
            m_navigatorOverlayButton->setToolTip(collapsed ? tr("Show navigator")
                                                          : tr("Hide navigator"));
        }
    });

    m_navigatorOverlayButton->setVisible(true);
    m_navigatorOverlayButton->setToolTip(m_navigator->isCollapsed()
        ? tr("Show navigator")
        : tr("Hide navigator"));
    positionNavigatorOverlayButton();
}

void MainWindow::setupHomeOverlayButton() {
    if (!m_viewport) {
        return;
    }

    m_homeOverlayButton = SidebarToolButton::fromSvgIcon(
        ":/icons/ic_home.svg",
        tr("Return to Home"),
        m_viewport
    );
    m_homeOverlayButton->setFixedSize(42, 42);
    m_homeOverlayButton->setVisible(true);

    connect(m_homeOverlayButton, &SidebarToolButton::clicked, this, [this]() {
        if (!m_startOverlay) {
            return;
        }

        if (!hasOpenProject()) {
            showStartDialog();
            return;
        }

        onSaveDocument();
        if (m_document && m_document->isModified()) {
            return;
        }

        resetDocumentState();
        showStartDialog();
    });

    positionHomeOverlayButton();
}

void MainWindow::positionHomeOverlayButton() {
    if (!m_viewport || !m_homeOverlayButton) {
        return;
    }

    const int margin = 20;
    m_homeOverlayButton->move(margin, margin);
    m_homeOverlayButton->raise();
}

void MainWindow::positionNavigatorOverlayButton() {
    if (!m_viewport || !m_navigatorOverlayButton) {
        return;
    }

    const int margin = 20;
    int y = margin;
    if (m_homeOverlayButton) {
        y = m_homeOverlayButton->y() + m_homeOverlayButton->height() + 10;
    }
    m_navigatorOverlayButton->move(margin, y);
    m_navigatorOverlayButton->raise();
}

void MainWindow::setupRenderDebugOverlay() {
    if (!m_viewport) {
        return;
    }

    m_renderDebugButton = new SidebarToolButton(QStringLiteral("D"),
                                                tr("Toggle render debug panel"),
                                                m_viewport);
    m_renderDebugButton->setFixedSize(42, 42);

    connect(m_renderDebugButton, &SidebarToolButton::clicked, this, [this]() {
        if (!m_renderDebugPanel) {
            return;
        }
        const bool visible = !m_renderDebugPanel->isVisible();
        m_renderDebugPanel->setVisible(visible);
        positionRenderDebugPanel();
    });

    m_renderDebugPanel = new RenderDebugPanel(m_viewport);
    m_renderDebugPanel->setVisible(false);

    connect(m_renderDebugPanel, &RenderDebugPanel::debugTogglesChanged, this, [this]() {
        if (!m_viewport || !m_renderDebugPanel) {
            return;
        }
        const auto toggles = m_renderDebugPanel->debugToggles();
        m_viewport->setDebugToggles(toggles.normals,
                                    toggles.depth,
                                    toggles.wireframe,
                                    toggles.disableGamma,
                                    toggles.matcap);
    });

    connect(m_renderDebugPanel, &RenderDebugPanel::lightRigChanged, this, [this]() {
        if (!m_viewport || !m_renderDebugPanel) {
            return;
        }
        const auto rig = m_renderDebugPanel->lightRig();
        m_viewport->setRenderLightRig(rig.keyDir,
                                      rig.fillDir,
                                      rig.fillIntensity,
                                      rig.ambientIntensity,
                                      rig.hemiUpDir,
                                      rig.gradientDir,
                                      rig.gradientStrength);
    });

    connect(m_renderDebugPanel, &RenderDebugPanel::resetToThemeRequested, this, [this]() {
        applyRenderDebugDefaults();
    });

    connect(m_viewport, &Viewport::debugTogglesChanged, this,
            [this](bool normals, bool depth, bool wireframe, bool disableGamma, bool matcap) {
                if (!m_renderDebugPanel) {
                    return;
                }
                RenderDebugPanel::DebugToggles toggles;
                toggles.normals = normals;
                toggles.depth = depth;
                toggles.wireframe = wireframe;
                toggles.disableGamma = disableGamma;
                toggles.matcap = matcap;
                m_renderDebugPanel->setDebugToggles(toggles);
            });

    RenderDebugPanel::DebugToggles toggles;
    toggles.normals = m_viewport->debugNormalsEnabled();
    toggles.depth = m_viewport->debugDepthEnabled();
    toggles.wireframe = m_viewport->wireframeOnlyEnabled();
    toggles.disableGamma = m_viewport->gammaDisabled();
    toggles.matcap = m_viewport->matcapEnabled();
    m_renderDebugPanel->setDebugToggles(toggles);

    applyRenderDebugDefaults();
    positionRenderDebugButton();
}

void MainWindow::positionRenderDebugButton() {
    if (!m_viewport || !m_renderDebugButton) {
        return;
    }
    const int margin = 20;
    int x = margin;
    int y = margin;
    if (m_navigatorOverlayButton) {
        y = m_navigatorOverlayButton->y() + m_navigatorOverlayButton->height() + 10;
    } else if (m_homeOverlayButton) {
        y = m_homeOverlayButton->y() + m_homeOverlayButton->height() + 10;
    }
    m_renderDebugButton->move(x, y);
    m_renderDebugButton->raise();
}

void MainWindow::positionRenderDebugPanel() {
    if (!m_viewport || !m_renderDebugPanel || !m_renderDebugPanel->isVisible()) {
        return;
    }
    const int margin = 20;
    int x = margin;
    int y = margin;
    if (m_renderDebugButton) {
        y = m_renderDebugButton->y() + m_renderDebugButton->height() + 10;
    }
    m_renderDebugPanel->move(x, y);
    m_renderDebugPanel->raise();
}

void MainWindow::applyRenderDebugDefaults() {
    if (!m_viewport || !m_renderDebugPanel) {
        return;
    }
    const auto& body = ThemeManager::instance().currentTheme().viewport.body;
    RenderDebugPanel::LightRig rig;
    rig.keyDir = body.keyLightDir;
    rig.fillDir = body.fillLightDir;
    rig.fillIntensity = body.fillLightIntensity;
    rig.ambientIntensity = body.ambientIntensity;
    rig.hemiUpDir = body.hemiUpDir;
    rig.gradientDir = body.ambientGradientDir;
    rig.gradientStrength = body.ambientGradientStrength;
    m_renderDebugPanel->setLightRig(rig);
    m_viewport->setRenderLightRig(rig.keyDir,
                                  rig.fillDir,
                                  rig.fillIntensity,
                                  rig.ambientIntensity,
                                  rig.hemiUpDir,
                                  rig.gradientDir,
                                  rig.gradientStrength);
}

void MainWindow::positionConstraintPanel() {
    if (!m_viewport || !m_constraintPanel) {
        return;
    }

    const int margin = 20;
    int x = m_viewport->width() - m_constraintPanel->width() - margin;
    int y = margin + 130;  // Below ViewCube
    m_constraintPanel->move(x, y);
    m_constraintPanel->raise();
}

void MainWindow::positionSketchModePanel() {
    if (!m_viewport || !m_sketchModePanel) {
        return;
    }

    const int margin = 20;
    // Position on right side, below constraint panel
    int x = m_viewport->width() - m_sketchModePanel->width() - margin;
    int y = margin + 130;  // Same starting position, panels stack vertically
    if (m_constraintPanel && m_constraintPanel->isVisible()) {
        y = m_constraintPanel->y() + m_constraintPanel->height() + 10;
    }
    m_sketchModePanel->move(x, y);
    m_sketchModePanel->raise();
}

void MainWindow::positionStartOverlay() {
    if (!m_startOverlay) {
        return;
    }
    QWidget* central = centralWidget();
    if (!central) {
        return;
    }
    m_startOverlay->setGeometry(central->rect());
    m_startOverlay->raise();
}

void MainWindow::setupViewport() {
    QWidget* central = new QWidget(this);
    QHBoxLayout* layout = new QHBoxLayout(central);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    m_centralLayout = layout;

    m_navigator = new ModelNavigator(central);
    m_navigator->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);

    m_viewport = new Viewport(central);
    m_viewport->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);

    layout->addWidget(m_navigator);
    layout->addWidget(m_viewport, 1);
    setCentralWidget(central);

    m_startOverlay = new StartOverlay(central);
    m_startOverlay->hide();
    positionStartOverlay();

    connect(m_startOverlay, &StartOverlay::newProjectRequested, this, [this]() {
        onNewDocument();
        if (m_startOverlay) {
            m_startOverlay->hide();
        }
    });

    connect(m_startOverlay, &StartOverlay::openProjectRequested, this, [this]() {
        onOpenDocument();
    });

    connect(m_startOverlay, &StartOverlay::recentProjectRequested, this, [this](const QString& path) {
        if (!maybeSave()) {
            return;
        }
        if (loadDocumentFromPath(path) && m_startOverlay) {
            m_startOverlay->hide();
        }
    });

    connect(m_startOverlay, &StartOverlay::deleteProjectRequested, this, [this](const QString& path) {
        if (path.isEmpty()) {
            return;
        }

        QFileInfo info(path);
        QString name = info.fileName();
        auto choice = QMessageBox::question(this, tr("Delete Project"),
            tr("Delete \"%1\" from disk? This cannot be undone.").arg(name),
            QMessageBox::Yes | QMessageBox::No, QMessageBox::No);
        if (choice != QMessageBox::Yes) {
            return;
        }

        if (!deleteProjectFromPath(path)) {
            QMessageBox::warning(this, tr("Delete Failed"),
                tr("Could not delete \"%1\".").arg(name));
            return;
        }

        QString resolvedPath = info.absoluteFilePath();
        if (!m_currentFilePath.isEmpty() && QFileInfo(m_currentFilePath).absoluteFilePath() == resolvedPath) {
            m_currentFilePath.clear();
            setWindowTitle(tr("OneCAD - Untitled"));
        }

        if (m_startOverlay) {
            m_startOverlay->setProjects(listProjectsInDefaultDirectory());
        }
    });

    central->installEventFilter(this);

    // Set document for rendering sketches in 3D mode
    m_viewport->setDocument(m_document.get());
    m_viewport->setCommandProcessor(m_commandProcessor.get());

    connect(m_viewport, &Viewport::mousePositionChanged,
            this, &MainWindow::onMousePositionChanged);
    connect(m_viewport, &Viewport::sketchModeChanged,
            this, &MainWindow::onSketchModeChanged);
    connect(m_viewport, &Viewport::sketchPlanePicked,
            this, &MainWindow::onSketchPlanePicked);
    connect(m_viewport, &Viewport::planeSelectionCancelled,
            this, &MainWindow::onPlaneSelectionCancelled);
    connect(m_viewport, &Viewport::sketchUpdated,
            this, &MainWindow::onSketchUpdated);
    connect(m_viewport, &Viewport::sketchSelectionChanged,
            this, &MainWindow::onSketchSelectionChanged);
    connect(m_viewport, &Viewport::openSketchForEditRequested,
            this, &MainWindow::openSketchForEdit);
    connect(m_viewport, &Viewport::statusMessageRequested, this, [this](const QString& message) {
        statusBar()->showMessage(message, 4000);
    });
    connect(m_viewport, &Viewport::constraintDeleteRequested,
            this, &MainWindow::onConstraintPanelDeleteRequested);
    connect(m_viewport, &Viewport::constraintSuppressRequested,
            this, &MainWindow::onConstraintPanelSuppressRequested);

    connect(m_navigator, &ModelNavigator::sketchSelected, this, [this](const QString& id) {
        if (m_viewport) {
            m_viewport->setReferenceSketch(id);
        }
    });
    connect(m_navigator, &ModelNavigator::editSketchRequested, this, &MainWindow::openSketchForEdit);

    setupNavigatorOverlayButton();
    setupHomeOverlayButton();
    setupRenderDebugOverlay();
    setupSnapOverlay(); // Before History panel to reserve space
    setupHistoryPanel();
    
    // Initial resize to position overlays
    positionHomeOverlayButton();
    positionNavigatorOverlayButton();

    // Create constraint panel (hidden initially)
    m_constraintPanel = new ConstraintPanel(m_viewport);
    m_constraintPanel->setVisible(false);
    connect(m_constraintPanel, &ConstraintPanel::constraintSelected,
            this, &MainWindow::onConstraintPanelConstraintSelected);
    connect(m_constraintPanel, &ConstraintPanel::constraintDeleteRequested,
            this, &MainWindow::onConstraintPanelDeleteRequested);
    connect(m_constraintPanel, &ConstraintPanel::constraintSuppressRequested,
            this, &MainWindow::onConstraintPanelSuppressRequested);
    connect(m_constraintPanel, &ConstraintPanel::restoreSuppressedRequested,
            this, &MainWindow::onConstraintPanelRestoreSuppressedRequested);

    // Create sketch mode panel (hidden initially)
    m_sketchModePanel = new SketchModePanel(m_viewport);
    m_sketchModePanel->setVisible(false);
    connect(m_sketchModePanel, &SketchModePanel::constraintRequested,
            this, &MainWindow::onConstraintRequested);

    // Setup history panel
    // setupHistoryPanel(); // Moved up

    m_viewport->installEventFilter(this);
}

void MainWindow::positionToolbarOverlay() {
    if (!m_viewport || !m_toolbar) {
        return;
    }

    const int margin = 20;
    const int availableWidth = m_viewport->width();
    const int toolbarWidth = m_toolbar->width();
    int xOffset = qMax(margin, (availableWidth - toolbarWidth) / 2);
    int yOffset = margin;

    m_toolbar->move(xOffset, yOffset);
    m_toolbar->raise();
}

bool MainWindow::eventFilter(QObject* obj, QEvent* event) {
    if (event->type() == QEvent::Resize) {
        if (obj == m_viewport || obj == centralWidget()) {
            if (m_startOverlay && m_startOverlay->isVisible()) {
                positionStartOverlay();
            }
        }
    }

    if (obj == m_viewport && event->type() == QEvent::Resize) {
        positionToolbarOverlay();
        positionHomeOverlayButton();
        positionNavigatorOverlayButton();
        positionRenderDebugButton();
        positionRenderDebugPanel();
        positionConstraintPanel();
        positionSketchModePanel();
        positionSnapOverlay();
        positionSnapSettingsPanel();
        positionHistoryPanel();
    }

    return QMainWindow::eventFilter(obj, event);
}

void MainWindow::setupStatusBar() {
    QStatusBar* status = statusBar();

    m_toolStatus = new QLabel(tr("Ready"));
    m_toolStatus->setMinimumWidth(150);
    status->addWidget(m_toolStatus);

    m_dofStatus = new QLabel(tr("DOF: —"));
    m_dofStatus->setMinimumWidth(80);
    status->addWidget(m_dofStatus);

    // Camera angle control (Shapr3D-style)
    QWidget* cameraAngleWidget = new QWidget(this);
    QHBoxLayout* cameraLayout = new QHBoxLayout(cameraAngleWidget);
    cameraLayout->setContentsMargins(10, 0, 10, 0);
    cameraLayout->setSpacing(8);

    QLabel* orthoLabel = new QLabel(tr("Orthographic"), cameraAngleWidget);
    orthoLabel->setStyleSheet("font-size: 10px;");

    m_cameraAngleSlider = new QSlider(Qt::Horizontal, cameraAngleWidget);
    m_cameraAngleSlider->setRange(0, 90);
    m_cameraAngleSlider->setValue(45);
    m_cameraAngleSlider->setFixedWidth(150);
    m_cameraAngleSlider->setTickPosition(QSlider::TicksBelow);
    m_cameraAngleSlider->setTickInterval(15);

    QLabel* perspLabel = new QLabel(tr("Perspective"), cameraAngleWidget);
    perspLabel->setStyleSheet("font-size: 10px;");

    m_cameraAngleLabel = new QLabel(tr("45°"), cameraAngleWidget);
    m_cameraAngleLabel->setMinimumWidth(35);
    m_cameraAngleLabel->setAlignment(Qt::AlignCenter);

    cameraLayout->addWidget(orthoLabel);
    cameraLayout->addWidget(m_cameraAngleSlider);
    cameraLayout->addWidget(perspLabel);
    cameraLayout->addWidget(m_cameraAngleLabel);

    status->addPermanentWidget(cameraAngleWidget);

    // Wire slider to camera
    connect(m_cameraAngleSlider, &QSlider::valueChanged, this, [this](int value) {
        m_cameraAngleLabel->setText(tr("%1°").arg(value));
        if (m_viewport) {
            m_viewport->setCameraAngle(static_cast<float>(value));
        }
    });

    m_coordStatus = new QLabel(tr("X: 0.00  Y: 0.00  Z: 0.00"));
    m_coordStatus->setMinimumWidth(200);
    status->addPermanentWidget(m_coordStatus);
}

void MainWindow::onNewSketch() {
    // If already in sketch mode, exit first (then continue create flow).
    if (m_viewport->isInSketchMode()) {
        onExitSketch();
    }

    m_activeSketchId.clear();

    const auto selection = m_viewport->modelSelection();
    if (selection.size() == 1 && selection[0].kind == app::selection::SelectionKind::Face) {
        const auto& selectedFace = selection[0];
        auto plane = m_document->getSketchPlaneForFace(selectedFace.id.ownerId,
                                                        selectedFace.id.elementId);
        if (plane) {
            auto sketch = std::make_unique<core::sketch::Sketch>(*plane);
            sketch->setHostFaceAttachment(selectedFace.id.ownerId, selectedFace.id.elementId);
            m_activeSketchId = m_document->addSketch(std::move(sketch));
            if (!m_activeSketchId.empty()) {
                m_document->ensureHostFaceBoundariesProjected(m_activeSketchId);
            }
            if (!m_activeSketchId.empty()) {
                m_viewport->setReferenceSketch(QString::fromStdString(m_activeSketchId));
            }

            core::sketch::Sketch* sketchPtr = m_document->getSketch(m_activeSketchId);
            if (sketchPtr) {
                m_viewport->enterSketchMode(sketchPtr);
                m_toolStatus->setText(tr("Sketch Mode - Selected Face"));
                m_toolbar->setContext(ContextToolbar::Context::Sketch);
                return;
            }

            m_activeSketchId.clear();
            m_toolStatus->setText(tr("Ready"));
            return;
        }
    }

    m_viewport->beginPlaneSelection();
    m_toolStatus->setText(tr("Select a plane to start sketch"));
}

void MainWindow::onSketchPlanePicked(int planeIndex) {
    core::sketch::SketchPlane plane;
    QString planeName;
    switch (planeIndex) {
        case 0:
            plane = core::sketch::SketchPlane::XY();
            planeName = "XY";
            break;
        case 1:
            plane = core::sketch::SketchPlane::XZ();
            planeName = "XZ";
            break;
        case 2:
        default:
            plane = core::sketch::SketchPlane::YZ();
            planeName = "YZ";
            break;
    }

    auto sketch = std::make_unique<core::sketch::Sketch>(plane);
    m_activeSketchId = m_document->addSketch(std::move(sketch));
    if (!m_activeSketchId.empty()) {
        m_viewport->setReferenceSketch(QString::fromStdString(m_activeSketchId));
    }

    core::sketch::Sketch* sketchPtr = m_document->getSketch(m_activeSketchId);
    if (!sketchPtr) {
        m_activeSketchId.clear();
        m_toolStatus->setText(tr("Ready"));
        return;
    }

    m_viewport->enterSketchMode(sketchPtr);
    m_toolStatus->setText(tr("Sketch Mode - %1 Plane").arg(planeName));
    m_toolbar->setContext(ContextToolbar::Context::Sketch);
}

void MainWindow::onPlaneSelectionCancelled() {
    if (!m_viewport->isInSketchMode()) {
        m_toolStatus->setText(tr("Ready"));
    }
}

void MainWindow::openSketchForEdit(const QString& sketchId) {
    if (sketchId.isEmpty() || !m_viewport || !m_document) {
        return;
    }
    const std::string id = sketchId.toStdString();
    m_viewport->setReferenceSketch(sketchId);
    m_activeSketchId = id;
    core::sketch::Sketch* sketchPtr = m_document->getSketch(m_activeSketchId);
    if (!sketchPtr) {
        m_activeSketchId.clear();
        return;
    }
    m_viewport->enterSketchMode(sketchPtr);
    QString sketchName = QString::fromStdString(m_document->getSketchName(id));
    m_toolStatus->setText(tr("Sketch Mode - %1").arg(sketchName));
    m_toolbar->setContext(ContextToolbar::Context::Sketch);
}

void MainWindow::onExitSketch() {
    if (!m_viewport->isInSketchMode()) return;

    // Exit sketch mode but keep sketch in document
    m_viewport->exitSketchMode();

    // Clear active sketch ID (we're no longer editing)
    m_activeSketchId.clear();

    m_toolStatus->setText(tr("Ready"));
    m_toolbar->setContext(ContextToolbar::Context::Default);

    // Trigger viewport update to show sketch in 3D view
    m_viewport->update();
}

void MainWindow::onSketchModeChanged(bool inSketchMode) {
    core::sketch::Sketch* activeSketch = nullptr;
    if (!m_activeSketchId.empty()) {
        activeSketch = m_document->getSketch(m_activeSketchId);
    }

    if (inSketchMode && activeSketch) {
        if (m_viewport && m_snapSettingsPanel) {
            m_viewport->updateSnapSettings(m_snapSettingsPanel->settings());
        }

        updateDofStatus(activeSketch);

        if (m_constraintPanel) {
            m_constraintPanel->setSketch(activeSketch);
            m_constraintPanel->setVisible(true);
            positionConstraintPanel();
        }

        if (m_sketchModePanel) {
            m_sketchModePanel->setSketch(activeSketch);
        }
        updateSketchConstraintUi();
    } else {
        updateDofStatus(nullptr);

        if (m_constraintPanel) {
            m_constraintPanel->setVisible(false);
        }

        if (m_sketchModePanel) {
            m_sketchModePanel->setVisible(false);
        }
    }
}

void MainWindow::onImport() {
    QString fileName = QFileDialog::getOpenFileName(this,
        tr("Import STEP File"), QString(),
        tr("STEP Files (*.step *.stp);;All Files (*)"));
    
    if (fileName.isEmpty()) return;
    
    m_toolStatus->setText(tr("Importing: %1").arg(QFileInfo(fileName).fileName()));
    auto result = io::StepImporter::import(fileName);
    
    if (!result.success) {
        QMessageBox::critical(this, tr("Import Failed"), result.errorMessage);
        m_toolStatus->setText(tr("Import failed"));
        return;
    }
    
    for (auto& body : result.bodies) {
        std::string bodyId = m_document->addBody(body.shape);
        if (!bodyId.empty()) {
            m_document->addBaseBodyId(bodyId);
        }
    }
    
    if (m_viewport) {
        m_viewport->update();
    }
    m_toolStatus->setText(tr("Imported %1 body(ies)").arg(result.bodies.size()));
}

void MainWindow::onExportStep() {
    auto bodyIds = m_document->getBodyIds();
    if (bodyIds.empty()) {
        QMessageBox::warning(this, tr("Export"), tr("No bodies to export."));
        return;
    }
    
    QString fileName = QFileDialog::getSaveFileName(this,
        tr("Export STEP File"), QString(),
        tr("STEP Files (*.step)"));
    
    if (fileName.isEmpty()) return;
    
    // Ensure .step extension
    if (!fileName.endsWith(".step", Qt::CaseInsensitive)) {
        fileName += ".step";
    }
    
    std::vector<TopoDS_Shape> shapes;
    for (const auto& id : bodyIds) {
        if (auto* s = m_document->getBodyShape(id)) {
            shapes.push_back(*s);
        }
    }
    
    auto result = io::StepExporter::exportShapes(fileName, shapes);
    if (!result.success) {
        QMessageBox::critical(this, tr("Export Failed"), result.errorMessage);
        return;
    }
    
    m_toolStatus->setText(tr("Exported %1 body(ies) to STEP").arg(shapes.size()));
}

bool MainWindow::maybeSave() {
    if (!m_document || !m_document->isModified()) {
        return true;
    }
    
    auto result = QMessageBox::warning(this, tr("Unsaved Changes"),
        tr("The document has been modified.\nDo you want to save your changes?"),
        QMessageBox::Save | QMessageBox::Discard | QMessageBox::Cancel,
        QMessageBox::Save);
    
    if (result == QMessageBox::Save) {
        onSaveDocument();
        return !m_document->isModified();  // Returns true if save succeeded
    }
    
    return result == QMessageBox::Discard;
}

void MainWindow::onNewDocument() {
    if (!maybeSave()) return;
    
    // Exit sketch mode if active
    if (m_viewport && m_viewport->isInSketchMode()) {
        onExitSketch();
    }
    
    m_document->clear();
    m_currentFilePath.clear();
    setWindowTitle(tr("OneCAD - Untitled"));
    m_toolStatus->setText(tr("New document"));
    if (m_startOverlay && m_startOverlay->isVisible()) {
        m_startOverlay->hide();
    }
    statusBar()->show();
}

QString MainWindow::defaultProjectDirectory() const {
    QString dir = QDir::homePath() + "/OneCAD/Projects";
    if (!QDir().mkpath(dir)) {
        qWarning() << "Failed to create project directory:" << dir;
        return QDir::homePath();
    }
    return dir;
}

bool MainWindow::loadDocumentFromPath(const QString& fileName) {
    if (fileName.isEmpty()) {
        return false;
    }

    QString resolvedPath = QFileInfo(fileName).absoluteFilePath();

    // Exit sketch mode if active
    if (m_viewport && m_viewport->isInSketchMode()) {
        onExitSketch();
    }

    m_toolStatus->setText(tr("Loading: %1").arg(QFileInfo(resolvedPath).fileName()));

    QString errorMessage;
    auto loadedDoc = io::OneCADFileIO::load(resolvedPath, errorMessage);
    if (!loadedDoc) {
        QMessageBox::critical(this, tr("Load Failed"), errorMessage);
        m_toolStatus->setText(tr("Load failed"));
        return false;
    }

    // Replace current document with loaded one
    m_document = std::move(loadedDoc);
    if (m_commandProcessor) {
        m_commandProcessor->clear();
    }
    m_viewport->setDocument(m_document.get());

    // Reconnect document signals to navigator
    connectDocumentSignals();

    if (m_historyPanel) {
        m_historyPanel->setDocument(m_document.get());
    }

    // Populate navigator with loaded data
    m_navigator->rebuild(m_document.get());

    m_currentFilePath = resolvedPath;
    setWindowTitle(tr("OneCAD - %1").arg(QFileInfo(resolvedPath).fileName()));
    m_toolStatus->setText(tr("Loaded successfully"));

    if (m_viewport) {
        m_viewport->update();
    }

    handleRegenerationFailures();

    if (m_startOverlay && m_startOverlay->isVisible()) {
        m_startOverlay->hide();
    }
    statusBar()->show();
    return true;
}

void MainWindow::handleRegenerationFailures() {
    if (!m_document) {
        return;
    }

    const auto& failures = m_document->operationFailures();
    if (failures.empty()) {
        return;
    }

    std::vector<RegenFailureDialog::FailedOp> failedOps;
    failedOps.reserve(failures.size());
    for (const auto& [opId, reason] : failures) {
        const app::OperationRecord* op = m_document->findOperation(opId);
        QString description = op ? formatOperationDisplayName(*op)
                                 : QString::fromStdString(opId);
        failedOps.push_back({
            QString::fromStdString(opId),
            description,
            QString::fromStdString(reason)
        });
    }

    RegenFailureDialog dialog(failedOps, this);
    if (dialog.exec() != QDialog::Accepted) {
        return;
    }

    auto action = dialog.selectedAction();
    std::vector<std::string> failedIds;
    failedIds.reserve(failures.size());
    for (const auto& [opId, reason] : failures) {
        failedIds.push_back(opId);
    }

    bool changed = false;
    if (action == RegenFailureDialog::Result::DeleteFailed) {
        for (const auto& opId : failedIds) {
            changed |= m_document->removeOperation(opId);
        }
    } else if (action == RegenFailureDialog::Result::SuppressFailed) {
        for (const auto& opId : failedIds) {
            changed |= m_document->setOperationSuppressed(opId, true);
        }
    } else {
        return;
    }

    if (changed) {
        app::history::RegenerationEngine regen(m_document.get());
        regen.regenerateAll();
        m_document->setModified(true);
        if (m_historyPanel) {
            m_historyPanel->rebuild();
        }
        if (m_viewport) {
            m_viewport->update();
        }
    }
}

void MainWindow::onOpenDocument() {
    if (!maybeSave()) return;
    
    QString fileName = QFileDialog::getOpenFileName(this,
        tr("Open OneCAD Project"), defaultProjectDirectory(),
        tr("OneCAD Files (*.onecad);;OneCAD Packages (*.onecadpkg);;All Files (*)"));
    
    if (fileName.isEmpty()) return;

    loadDocumentFromPath(fileName);
}

bool MainWindow::saveDocumentToPath(const QString& filePath) {
    if (!m_document) {
        return false;
    }

    if (m_toolStatus) {
        m_toolStatus->setText(tr("Saving..."));
    }

    // Capture viewport thumbnail
    QImage thumbnail;
    if (m_viewport) {
        thumbnail = m_viewport->captureThumbnail(512);
    }

    auto result = io::OneCADFileIO::save(filePath, m_document.get(), thumbnail);
    if (!result.success) {
        QMessageBox::critical(this, tr("Save Failed"), result.errorMessage);
        if (m_toolStatus) {
            m_toolStatus->setText(tr("Save failed"));
        }
        return false;
    }

    m_document->setModified(false);
    if (m_toolStatus) {
        m_toolStatus->setText(tr("Saved"));
    }
    return true;
}

bool MainWindow::hasOpenProject() const {
    if (!m_document) {
        return false;
    }

    if (!m_currentFilePath.isEmpty()) {
        return true;
    }

    if (m_document->isModified()) {
        return true;
    }

    return m_document->sketchCount() > 0 || m_document->bodyCount() > 0;
}

void MainWindow::resetDocumentState() {
    if (m_viewport && m_viewport->isInSketchMode()) {
        onExitSketch();
    }

    if (m_commandProcessor) {
        m_commandProcessor->clear();
    }

    if (m_document) {
        m_document->clear();
    }

    m_activeSketchId.clear();
    m_currentFilePath.clear();
    setWindowTitle(tr("OneCAD - Untitled"));
    if (m_toolStatus) {
        m_toolStatus->setText(tr("Ready"));
    }

    if (m_viewport) {
        m_viewport->update();
    }
}

void MainWindow::onSaveDocument() {
    QString targetPath = m_currentFilePath;
    if (targetPath.isEmpty()) {
        bool ok = false;
        QString name = QInputDialog::getText(
            this,
            tr("Save Project"),
            tr("Project name:"),
            QLineEdit::Normal,
            tr("Untitled"),
            &ok);

        if (!ok) {
            return;
        }

        name = name.trimmed();
        if (name.isEmpty()) {
            QMessageBox::warning(this, tr("Save Project"), tr("Project name cannot be empty."));
            return;
        }

        name.replace("/", "-");
        name.replace("\\", "-");

        targetPath = defaultProjectDirectory() + "/" + name;
        if (!targetPath.endsWith(".onecad", Qt::CaseInsensitive)) {
            targetPath += ".onecad";
        }

        QFileInfo fileInfo(targetPath);
        if (fileInfo.exists()) {
            auto choice = QMessageBox::question(this, tr("Overwrite Project"),
                tr("A project with this name already exists. Overwrite it?"),
                QMessageBox::Yes | QMessageBox::No, QMessageBox::No);
            if (choice != QMessageBox::Yes) {
                return;
            }
        }
    }
    
    if (!saveDocumentToPath(targetPath)) {
        return;
    }

    m_currentFilePath = targetPath;
    setWindowTitle(tr("OneCAD - %1").arg(QFileInfo(targetPath).fileName()));
}

void MainWindow::onSaveDocumentAs() {
    QString fileName = QFileDialog::getSaveFileName(this,
        tr("Save OneCAD Project As"), defaultProjectDirectory() + "/Untitled.onecad",
        tr("OneCAD Files (*.onecad)"));
    
    if (fileName.isEmpty()) return;
    
    // Ensure .onecad extension
    if (!fileName.endsWith(".onecad", Qt::CaseInsensitive)) {
        fileName += ".onecad";
    }
    
    if (!saveDocumentToPath(fileName)) {
        return;
    }

    m_currentFilePath = fileName;
    setWindowTitle(tr("OneCAD - %1").arg(QFileInfo(fileName).fileName()));
}

void MainWindow::showStartDialog() {
    if (!m_startOverlay) {
        return;
    }
    m_startOverlay->setProjects(listProjectsInDefaultDirectory());
    statusBar()->hide();
    m_startOverlay->show();
    positionStartOverlay();
    m_startOverlay->raise();
    m_startOverlay->setFocus(Qt::OtherFocusReason);
}

bool MainWindow::deleteProjectFromPath(const QString& filePath) {
    QFileInfo info(filePath);
    if (!info.exists()) {
        return true;
    }

    if (info.isDir()) {
        QDir dir(info.absoluteFilePath());
        return dir.removeRecursively();
    }

    return QFile::remove(info.absoluteFilePath());
}

QStringList MainWindow::listProjectsInDefaultDirectory() const {
    QDir dir(defaultProjectDirectory());
    QList<QFileInfo> entries;

    QFileInfoList files = dir.entryInfoList(QStringList() << "*.onecad",
                                            QDir::Files | QDir::NoDotAndDotDot);
    QFileInfoList packages = dir.entryInfoList(QStringList() << "*.onecadpkg",
                                               QDir::Dirs | QDir::NoDotAndDotDot);

    entries.append(files);
    entries.append(packages);

    std::sort(entries.begin(), entries.end(),
              [](const QFileInfo& a, const QFileInfo& b) {
                  return a.lastModified() > b.lastModified();
              });

    QStringList paths;
    for (const QFileInfo& info : entries) {
        paths.append(info.absoluteFilePath());
    }

    return paths;
}

void MainWindow::onMousePositionChanged(double x, double y, double z) {
    m_coordStatus->setText(tr("X: %1  Y: %2  Z: %3")
        .arg(x, 0, 'f', 2)
        .arg(y, 0, 'f', 2)
        .arg(z, 0, 'f', 2));
}

void MainWindow::onSketchUpdated() {
    if (!m_viewport->isInSketchMode()) return;

    core::sketch::Sketch* sketch = m_viewport->activeSketch();
    if (!sketch) return;

    updateDofStatus(sketch);
    updateSketchConstraintUi();
}

void MainWindow::onSketchSelectionChanged() {
    updateSketchConstraintUi();
}

void MainWindow::updateSketchConstraintUi() {
    if (!m_viewport || !m_viewport->isInSketchMode()) {
        return;
    }

    core::sketch::Sketch* sketch = m_viewport->activeSketch();
    if (!sketch) {
        return;
    }

    const auto selection = m_viewport->sketchSelection();
    const auto applicability = evaluateConstraintApplicability(sketch, selection);

    std::vector<std::string> selectedEntityIds;
    selectedEntityIds.reserve(selection.size());
    QString selectedConstraintId;
    std::unordered_set<std::string> seenEntityIds;
    for (const auto& item : selection) {
        if (item.kind == app::selection::SelectionKind::SketchConstraint && selectedConstraintId.isEmpty()) {
            selectedConstraintId = QString::fromStdString(item.id.elementId);
            continue;
        }
        if (item.kind == app::selection::SelectionKind::SketchPoint ||
            item.kind == app::selection::SelectionKind::SketchEdge) {
            if (!item.id.elementId.empty() && seenEntityIds.insert(item.id.elementId).second) {
                selectedEntityIds.push_back(item.id.elementId);
            }
        }
    }

    if (m_constraintPanel) {
        const int conflicts = static_cast<int>(sketch->getConflictingConstraints().size());
        const int suppressedCount = m_viewport->suppressedConstraintMarkerCount();
        m_constraintPanel->setSketch(sketch);
        m_constraintPanel->setContext(selectedEntityIds,
                                      selectedConstraintId,
                                      sketch->getDegreesOfFreedom(),
                                      conflicts,
                                      suppressedCount);
        m_constraintPanel->setVisible(true);
        positionConstraintPanel();
    }

    if (m_sketchModePanel) {
        m_sketchModePanel->setSketch(sketch);
        m_sketchModePanel->setApplicableConstraints(applicability.applicableConstraints);
        const bool showTools = applicability.hasApplicableConstraints();
        m_sketchModePanel->setVisible(showTools);
        if (showTools) {
            positionSketchModePanel();
        } else if (m_toolStatus) {
            m_toolStatus->setText(tr("Select geometry to constrain"));
        }
    }
}

void MainWindow::onConstraintPanelConstraintSelected(const QString& constraintId) {
    if (!m_viewport || !m_viewport->isInSketchMode() || constraintId.isEmpty()) {
        return;
    }
    m_viewport->selectSketchConstraint(constraintId);
}

void MainWindow::onConstraintPanelDeleteRequested(const QString& constraintId) {
    if (!m_viewport || !m_viewport->isInSketchMode() || constraintId.isEmpty()) {
        return;
    }

    core::sketch::Sketch* sketch = m_viewport->activeSketch();
    auto* renderer = m_viewport->sketchRenderer();
    if (!sketch || !renderer) {
        return;
    }

    const bool removed = sketch->removeConstraint(constraintId.toStdString());
    if (!removed) {
        m_toolStatus->setText(tr("Constraint could not be removed"));
        return;
    }

    m_viewport->unsuppressConstraintMarker(constraintId);

    auto result = sketch->solve();
    renderer->updateGeometry();
    renderer->updateConstraints();
    m_viewport->notifySketchUpdated();
    m_viewport->update();
    m_toolStatus->setText(result.success ? tr("Constraint removed")
                                         : tr("Constraint removed - solver failed"));
}

void MainWindow::onConstraintPanelSuppressRequested(const QString& constraintId) {
    if (!m_viewport || !m_viewport->isInSketchMode() || constraintId.isEmpty()) {
        return;
    }

    m_viewport->suppressConstraintMarker(constraintId);
    m_toolStatus->setText(tr("Constraint marker suppressed"));
    updateSketchConstraintUi();
}

void MainWindow::onConstraintPanelRestoreSuppressedRequested() {
    if (!m_viewport || !m_viewport->isInSketchMode()) {
        return;
    }
    m_viewport->clearSuppressedConstraintMarkers();
    m_toolStatus->setText(tr("Constraint markers restored"));
    updateSketchConstraintUi();
}

void MainWindow::onConstraintRequested(core::sketch::ConstraintType constraintType) {
    if (!m_viewport || !m_viewport->isInSketchMode()) {
        return;
    }

    core::sketch::Sketch* sketch = m_viewport->activeSketch();
    if (!sketch) {
        return;
    }

    // Get selected entities from renderer
    auto* renderer = m_viewport->sketchRenderer();
    if (!renderer) {
        return;
    }

    std::vector<core::sketch::EntityID> selected = renderer->getSelectedEntities();

    using CT = core::sketch::ConstraintType;
    CT type = constraintType;
    core::sketch::ConstraintID constraintId;

    switch (type) {
        case CT::Horizontal:
            if (selected.size() == 1) {
                constraintId = sketch->addHorizontal(selected[0]);
            } else if (selected.size() == 2) {
                constraintId = sketch->addHorizontal(selected[0], selected[1]);
            }
            break;

        case CT::Vertical:
            if (selected.size() == 1) {
                constraintId = sketch->addVertical(selected[0]);
            } else if (selected.size() == 2) {
                constraintId = sketch->addVertical(selected[0], selected[1]);
            }
            break;

        case CT::Parallel:
            if (selected.size() == 2) {
                constraintId = sketch->addParallel(selected[0], selected[1]);
            } else {
                m_toolStatus->setText(tr("Parallel requires exactly 2 lines"));
                return;
            }
            break;

        case CT::Perpendicular:
            if (selected.size() == 2) {
                constraintId = sketch->addPerpendicular(selected[0], selected[1]);
            } else {
                m_toolStatus->setText(tr("Perpendicular requires exactly 2 lines"));
                return;
            }
            break;

        case CT::Coincident:
            if (selected.size() == 2) {
                constraintId = sketch->addCoincident(selected[0], selected[1]);
            } else {
                m_toolStatus->setText(tr("Coincident requires exactly 2 points"));
                return;
            }
            break;

        case CT::Fixed:
            if (selected.size() == 1) {
                constraintId = sketch->addFixed(selected[0]);
            } else {
                m_toolStatus->setText(tr("Fixed requires exactly 1 point"));
                return;
            }
            break;

        case CT::Distance:
            if (selected.size() == 2) {
                constraintId = sketch->addDistance(selected[0], selected[1], kDefaultDistanceMm);
            } else {
                m_toolStatus->setText(tr("Distance requires exactly 2 entities"));
                return;
            }
            break;

        case CT::Angle:
            if (selected.size() == 2) {
                constraintId = sketch->addAngle(selected[0], selected[1], kDefaultAngleDeg);
            } else {
                m_toolStatus->setText(tr("Angle requires exactly 2 lines"));
                return;
            }
            break;

        case CT::Radius:
            if (selected.size() == 1) {
                constraintId = sketch->addRadius(selected[0], kDefaultRadiusMm);
            } else {
                m_toolStatus->setText(tr("Radius requires exactly 1 circle or arc"));
                return;
            }
            break;

        case CT::OnCurve: {
            // Need exactly 1 point and 1 curve
            core::sketch::EntityID pointId, curveId;
            for (const auto& id : selected) {
                auto* entity = sketch->getEntity(id);
                if (!entity) continue;

                if (entity->type() == core::sketch::EntityType::Point) {
                    if (!pointId.empty()) {
                        m_toolStatus->setText(tr("Point On Curve requires exactly 1 point"));
                        return;
                    }
                    pointId = id;
                } else if (entity->type() == core::sketch::EntityType::Arc ||
                           entity->type() == core::sketch::EntityType::Circle ||
                           entity->type() == core::sketch::EntityType::Ellipse ||
                           entity->type() == core::sketch::EntityType::Line) {
                    if (!curveId.empty()) {
                        m_toolStatus->setText(tr("Point On Curve requires exactly 1 curve"));
                        return;
                    }
                    curveId = id;
                }
            }

            if (pointId.empty() || curveId.empty()) {
                m_toolStatus->setText(tr("Point On Curve requires 1 point and 1 curve"));
                return;
            }

            // Auto-detect position (Start/End/Arbitrary)
            constraintId = sketch->addPointOnCurve(pointId, curveId);
            break;
        }

        default:
            // Other constraint types not yet implemented
            m_toolStatus->setText(tr("Constraint type not implemented"));
            return;
    }

    if (!constraintId.empty()) {
        // Solve and update
        auto result = sketch->solve();
        if (result.success) {
            renderer->updateGeometry();
            renderer->updateConstraints();
            m_viewport->notifySketchUpdated();
            m_viewport->update();
            m_toolStatus->setText(tr("Constraint applied"));
        } else {
            // Solver failed - show error to user
            m_toolStatus->setText(tr("Constraint applied - solver failed (over-constrained or conflicting)"));
            // Note: constraint was still added to sketch, just not solved
            // Could optionally remove the constraint here if desired
        }
    } else {
        if (type == CT::Fixed && selected.size() == 1) {
            m_toolStatus->setText(tr("Fixed requires a point (select a point, not an edge)"));
        } else {
            m_toolStatus->setText(tr("Cannot apply constraint to selection"));
        }
    }

    updateSketchConstraintUi();
}

void MainWindow::onDeleteItem(const QString& itemId) {
    std::string id = itemId.toStdString();

    // Determine item type
    bool isBody = m_document->getBodyShape(id) != nullptr;
    bool isSketch = m_document->getSketch(id) != nullptr;

    if (!isBody && !isSketch) {
        return;
    }

    // Get item name for confirmation
    QString itemName = isBody
        ? QString::fromStdString(m_document->getBodyName(id))
        : QString::fromStdString(m_document->getSketchName(id));

    // Always show confirmation dialog
    if (QMessageBox::question(this, tr("Confirm Delete"),
            tr("Delete '%1'?").arg(itemName),
            QMessageBox::Yes | QMessageBox::No) != QMessageBox::Yes) {
        return;
    }

    // If deleting active sketch, exit sketch mode first
    if (isSketch && m_activeSketchId == id) {
        onExitSketch();
    }

    // Create and execute command
    if (isBody) {
        auto cmd = std::make_unique<app::commands::DeleteBodyCommand>(m_document.get(), id);
        m_commandProcessor->execute(std::move(cmd));
    } else {
        auto cmd = std::make_unique<app::commands::DeleteSketchCommand>(m_document.get(), id);
        m_commandProcessor->execute(std::move(cmd));
    }

    m_viewport->update();
}

void MainWindow::onRenameItem(const QString& itemId) {
    // Trigger inline edit in navigator
    m_navigator->startInlineEdit(itemId);
}

void MainWindow::onVisibilityToggled(const QString& itemId, bool visible) {
    std::string id = itemId.toStdString();
    bool isBody = m_document->getBodyShape(id) != nullptr;

    auto cmd = std::make_unique<app::commands::ToggleVisibilityCommand>(
        m_document.get(), id,
        isBody ? app::commands::ToggleVisibilityCommand::ItemType::Body
               : app::commands::ToggleVisibilityCommand::ItemType::Sketch,
        visible);
    m_commandProcessor->execute(std::move(cmd));

    m_viewport->update();
}

void MainWindow::onIsolateItem(const QString& itemId) {
    std::string id = itemId.toStdString();

    // Toggle isolation
    if (m_document->isolatedItemId() == id) {
        m_document->clearIsolation();
    } else {
        m_document->isolateItem(id);
    }

    m_viewport->update();
}

void MainWindow::loadSettings() {
    QSettings settings("OneCAD", "OneCAD");

    // Restore saved camera angle (default to 45 degrees if not set)
    float savedAngle = settings.value("viewport/cameraAngle", 45.0f).toFloat();

    if (m_cameraAngleSlider) {
        m_cameraAngleSlider->setValue(static_cast<int>(savedAngle));
    }

    if (m_viewport) {
        m_viewport->setCameraAngle(savedAngle);
    }
}

void MainWindow::saveSettings() {
    QSettings settings("OneCAD", "OneCAD");

    // Save current camera angle
    if (m_viewport && m_viewport->camera()) {
        float currentAngle = m_viewport->camera()->cameraAngle();
        settings.setValue("viewport/cameraAngle", currentAngle);
    }

    settings.sync(); // Force immediate write
}

void MainWindow::setupSnapOverlay() {
    if (!m_viewport) return;

    m_snapSettingsButton = new SidebarToolButton(":/icons/ic_snap.svg", tr("Snap Settings"), m_viewport);
    m_snapSettingsButton->setFixedSize(42, 42);
    m_snapSettingsButton->setVisible(true);

    m_snapSettingsPanel = new SnapSettingsPanel(m_viewport);
    m_snapSettingsPanel->setVisible(false);

    connect(m_snapSettingsButton, &SidebarToolButton::clicked, this, [this]() {
        if (m_snapSettingsPanel) {
            m_snapSettingsPanel->setVisible(!m_snapSettingsPanel->isVisible());
            positionSnapSettingsPanel();
        }
    });
    
    // Connect settings changes to viewport
    connect(m_snapSettingsPanel, &SnapSettingsPanel::settingsChanged, this, [this]() {
        if (m_viewport && m_snapSettingsPanel) {
            m_viewport->updateSnapSettings(m_snapSettingsPanel->settings());
        }
    });

    positionSnapOverlay();
}

void MainWindow::positionSnapOverlay() {
    if (!m_viewport || !m_snapSettingsButton) return;

    const int margin = 20;
    int x = m_viewport->width() - m_snapSettingsButton->width() - margin;
    int y = margin;
    m_snapSettingsButton->move(x, y);
    m_snapSettingsButton->raise();
}

void MainWindow::positionSnapSettingsPanel() {
    if (!m_viewport || !m_snapSettingsPanel || !m_snapSettingsButton) return;
    
    if (!m_snapSettingsPanel->isVisible()) return;

    const int margin = 10;
    int x = m_snapSettingsButton->x() - m_snapSettingsPanel->width() - margin;
    int y = m_snapSettingsButton->y();
    
    // Ensure it doesn't go off screen
    if (x < margin) x = margin;
    
    m_snapSettingsPanel->move(x, y);
    m_snapSettingsPanel->raise();
}

void MainWindow::setupHistoryPanel() {
    if (!m_viewport) return;

    // Create history panel (right sidebar)
    QWidget* parent = centralWidget() ? centralWidget() : m_viewport;
    m_historyPanel = new HistoryPanel(parent);
    m_historyPanel->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);
    if (m_centralLayout) {
        m_centralLayout->addWidget(m_historyPanel);
    }
    m_historyPanel->setDocument(m_document.get());
    m_historyPanel->setViewport(m_viewport);
    m_historyPanel->setCommandProcessor(m_commandProcessor.get());

    connect(m_historyPanel, &HistoryPanel::rollbackRequested, this, [this](const QString& opId) {
        auto cmd = std::make_unique<app::commands::RollbackCommand>(
            m_document.get(), opId.toStdString());
        m_commandProcessor->execute(std::move(cmd));
        if (m_viewport) {
            m_viewport->update();
        }
    });

    connect(m_historyPanel, &HistoryPanel::suppressRequested, this,
            [this](const QString& opId, bool suppress) {
                auto cmd = std::make_unique<app::commands::SetOperationSuppressionCommand>(
                    m_document.get(), opId.toStdString(), suppress);
                m_commandProcessor->execute(std::move(cmd));
                if (m_viewport) {
                    m_viewport->update();
                }
            });

    connect(m_historyPanel, &HistoryPanel::deleteRequested, this, [this](const QString& opId) {
        auto cmd = std::make_unique<app::commands::RemoveOperationCommand>(
            m_document.get(), opId.toStdString());
        m_commandProcessor->execute(std::move(cmd));
        if (m_viewport) {
            m_viewport->update();
        }
    });

    // Create toggle button
    m_historyOverlayButton = new SidebarToolButton("H", tr("Toggle History Panel (Cmd+H)"), m_viewport);
    m_historyOverlayButton->setFixedSize(42, 42);
    m_historyOverlayButton->setVisible(true);

    connect(m_historyOverlayButton, &SidebarToolButton::clicked, this, [this]() {
        if (m_historyPanel) {
            m_historyPanel->setCollapsed(!m_historyPanel->isCollapsed());
        }
    });

    connect(m_historyPanel, &HistoryPanel::collapsedChanged, this, [this](bool collapsed) {
        if (m_historyOverlayButton) {
      // Right side
    positionSnapOverlay();
    positionSnapSettingsPanel();
    positionHistoryPanel();
            m_historyOverlayButton->setToolTip(collapsed ? tr("Show history")
                                                         : tr("Hide history"));
        }
        QSettings settings("OneCAD", "OneCAD");
        settings.setValue("ui/historyPanelVisible", !collapsed);
    });

    auto* shortcut = new QShortcut(QKeySequence("Ctrl+H"), m_viewport);
    connect(shortcut, &QShortcut::activated, this, [this]() {
        if (m_historyPanel) {
            m_historyPanel->setCollapsed(!m_historyPanel->isCollapsed());
        }
    });

    QSettings settings("OneCAD", "OneCAD");
    const bool historyVisible = settings.value("ui/historyPanelVisible", false).toBool();
    m_historyPanel->setCollapsed(!historyVisible);
    m_historyOverlayButton->setToolTip(historyVisible ? tr("Hide history")
                                                      : tr("Show history"));

    positionHistoryPanel();
}

void MainWindow::positionHistoryPanel() {
    if (!m_viewport || !m_historyOverlayButton) return;

    const int margin = 20;
    int x = m_viewport->width() - m_historyOverlayButton->width() - margin;
    int y = margin;
    
    // Position below Snap button if it exists
    if (m_snapSettingsButton && m_snapSettingsButton->isVisible()) {
        y = m_snapSettingsButton->y() + m_snapSettingsButton->height() + 10;
    }

    m_historyOverlayButton->move(x, y);
    m_historyOverlayButton->raise();
}

} // namespace ui
} // namespace onecad
