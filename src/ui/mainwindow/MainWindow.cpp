#include "MainWindow.h"
#include "../theme/ThemeManager.h"
#include "../viewport/Viewport.h"
#include "../../render/Camera3D.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/document/Document.h"
#include "../navigator/ModelNavigator.h"
#include "../toolbar/ContextToolbar.h"
#include "../sketch/ConstraintPanel.h"
#include "../sketch/SketchModePanel.h"
#include "../../core/sketch/SketchRenderer.h"
#include "../../core/sketch/SketchTypes.h"
#include <QMenuBar>
#include <QStatusBar>
#include <QLabel>
#include <QSlider>
#include <QApplication>
#include <QMessageBox>
#include <QFileDialog>
#include <QActionGroup>
#include <QSettings>
#include <QHBoxLayout>
#include <QWidget>
#include <QSizePolicy>
#include <QEvent>

#include "../components/SidebarToolButton.h"

namespace onecad {
namespace ui {

namespace {
// Default constraint values
constexpr double kDefaultDistanceMm = 10.0;
constexpr double kDefaultAngleDeg = 90.0;
constexpr double kDefaultRadiusMm = 10.0;
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

    // Connect document signals to navigator
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

    loadSettings();
}

MainWindow::~MainWindow() {
    saveSettings();
}

void MainWindow::applyTheme() {
    ThemeManager::instance().applyTheme();
    applyDofStatusStyle();
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
    fileMenu->addAction(tr("&New"), QKeySequence::New, this, []() {});
    fileMenu->addAction(tr("&Open..."), QKeySequence::Open, this, []() {});
    fileMenu->addSeparator();
    fileMenu->addAction(tr("&Save"), QKeySequence::Save, this, []() {});
    fileMenu->addAction(tr("Save &As..."), QKeySequence::SaveAs, this, []() {});
    fileMenu->addSeparator();
    fileMenu->addAction(tr("&Import STEP..."), this, &MainWindow::onImport);
    fileMenu->addAction(tr("&Export STEP..."), this, []() {});
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
    connect(m_toolbar, &ContextToolbar::exitSketchRequested,
            this, &MainWindow::onExitSketch);
    connect(m_toolbar, &ContextToolbar::importRequested,
            this, &MainWindow::onImport);

    if (m_viewport) {
        connect(m_toolbar, &ContextToolbar::lineToolActivated,
                m_viewport, &Viewport::activateLineTool);
        connect(m_toolbar, &ContextToolbar::circleToolActivated,
                m_viewport, &Viewport::activateCircleTool);
        connect(m_toolbar, &ContextToolbar::rectangleToolActivated,
                m_viewport, &Viewport::activateRectangleTool);
        connect(m_toolbar, &ContextToolbar::arcToolActivated,
                m_viewport, &Viewport::activateArcTool);
        connect(m_toolbar, &ContextToolbar::ellipseToolActivated,
                m_viewport, &Viewport::activateEllipseTool);
        connect(m_toolbar, &ContextToolbar::trimToolActivated,
                m_viewport, &Viewport::activateTrimTool);
        connect(m_toolbar, &ContextToolbar::mirrorToolActivated,
                m_viewport, &Viewport::activateMirrorTool);

        connect(m_viewport, &Viewport::extrudeToolActiveChanged,
                m_toolbar, &ContextToolbar::setExtrudeActive);
        connect(m_viewport, &Viewport::revolveToolActiveChanged,
                m_toolbar, &ContextToolbar::setRevolveActive);
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

void MainWindow::positionNavigatorOverlayButton() {
    if (!m_viewport || !m_navigatorOverlayButton) {
        return;
    }

    const int margin = 20;
    m_navigatorOverlayButton->move(margin, margin);
    m_navigatorOverlayButton->raise();
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

void MainWindow::setupViewport() {
    QWidget* central = new QWidget(this);
    QHBoxLayout* layout = new QHBoxLayout(central);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);

    m_navigator = new ModelNavigator(central);
    m_navigator->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);

    m_viewport = new Viewport(central);
    m_viewport->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);

    layout->addWidget(m_navigator);
    layout->addWidget(m_viewport, 1);
    setCentralWidget(central);

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

    connect(m_navigator, &ModelNavigator::sketchSelected, this, [this](const QString& id) {
        if (m_viewport) {
            m_viewport->setReferenceSketch(id);
        }
    });

    setupNavigatorOverlayButton();

    // Create constraint panel (hidden initially)
    m_constraintPanel = new ConstraintPanel(m_viewport);
    m_constraintPanel->setVisible(false);

    // Create sketch mode panel (hidden initially)
    m_sketchModePanel = new SketchModePanel(m_viewport);
    m_sketchModePanel->setVisible(false);
    connect(m_sketchModePanel, &SketchModePanel::constraintRequested,
            this, &MainWindow::onConstraintRequested);

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
    if (obj == m_viewport && event->type() == QEvent::Resize) {
        positionToolbarOverlay();
        positionNavigatorOverlayButton();
        positionConstraintPanel();
        positionSketchModePanel();
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
    // If already in sketch mode, exit first
    if (m_viewport->isInSketchMode()) {
        onExitSketch();
    }

    m_activeSketchId.clear();
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
        updateDofStatus(activeSketch);

        // Show constraint panel
        if (m_constraintPanel) {
            m_constraintPanel->setSketch(activeSketch);
            m_constraintPanel->setVisible(true);
            positionConstraintPanel();
        }

        // Show sketch mode panel
        if (m_sketchModePanel) {
            m_sketchModePanel->setSketch(activeSketch);
            m_sketchModePanel->setVisible(true);
            positionSketchModePanel();
        }
    } else {
        updateDofStatus(nullptr);

        // Hide constraint panel
        if (m_constraintPanel) {
            m_constraintPanel->setVisible(false);
        }

        // Hide sketch mode panel
        if (m_sketchModePanel) {
            m_sketchModePanel->setVisible(false);
        }
    }
}

void MainWindow::onImport() {
    QString fileName = QFileDialog::getOpenFileName(this,
        tr("Import STEP File"), QString(),
        tr("STEP Files (*.step *.stp);;All Files (*)"));
    
    if (!fileName.isEmpty()) {
        m_toolStatus->setText(tr("Importing: %1").arg(fileName));
        // TODO: Actual import
    }
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

    // Refresh constraint panel
    if (m_constraintPanel) {
        m_constraintPanel->refresh();
    }
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
        m_toolStatus->setText(tr("Cannot apply constraint to selection"));
    }
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

} // namespace ui
} // namespace onecad
