#include "MainWindow.h"
#include "../theme/ThemeManager.h"
#include "../viewport/Viewport.h"
#include "../../render/Camera3D.h"
#include "../../core/sketch/Sketch.h"
#include "../../core/sketch/tools/SketchToolManager.h"
#include "../../app/document/Document.h"
#include "../navigator/ModelNavigator.h"
#include "../toolbar/ContextToolbar.h"
#include "../sketch/ConstraintPanel.h"

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

MainWindow::MainWindow(QWidget* parent)
    : QMainWindow(parent) {

    setWindowTitle(tr("OneCAD"));
    resize(1280, 800);
    setMinimumSize(800, 600);

    // Create document model (no Qt parent - unique_ptr manages lifetime)
    m_document = std::make_unique<app::Document>();

    applyTheme();
    setupMenuBar();
    setupViewport();
    setupToolBar();
    setupStatusBar();

    // Connect document signals to navigator
    connect(m_document.get(), &app::Document::sketchAdded,
            m_navigator, &ModelNavigator::onSketchAdded);
    connect(m_document.get(), &app::Document::sketchRemoved,
            m_navigator, &ModelNavigator::onSketchRemoved);
    connect(m_document.get(), &app::Document::sketchRenamed,
            m_navigator, &ModelNavigator::onSketchRenamed);

    loadSettings();
}

MainWindow::~MainWindow() {
    saveSettings();
}

void MainWindow::applyTheme() {
    ThemeManager::instance().applyTheme();
}

void MainWindow::updateDofStatus(core::sketch::Sketch* sketch) {
    if (!m_dofStatus) {
        return;
    }

    if (!sketch) {
        m_dofStatus->setText(tr("DOF: —"));
        m_dofStatus->setStyleSheet("");
        return;
    }

    int dof = sketch->getDegreesOfFreedom();
    bool overConstrained = sketch->isOverConstrained();
    m_dofStatus->setText(tr("DOF: %1").arg(dof));

    if (overConstrained) {
        m_dofStatus->setStyleSheet("color: red;");
    } else if (dof == 0) {
        m_dofStatus->setStyleSheet("color: green;");
    } else {
        m_dofStatus->setStyleSheet("color: orange;");
    }
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
    editMenu->addAction(tr("&Undo"), QKeySequence::Undo, this, []() {});
    editMenu->addAction(tr("&Redo"), QKeySequence::Redo, this, []() {});
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
    
    QAction* lightAction = themeMenu->addAction(tr("&Light"));
    lightAction->setCheckable(true);
    connect(lightAction, &QAction::triggered, this, [](){
        ThemeManager::instance().setThemeMode(ThemeManager::ThemeMode::Light);
    });

    QAction* darkAction = themeMenu->addAction(tr("&Dark"));
    darkAction->setCheckable(true);
    connect(darkAction, &QAction::triggered, this, [](){
        ThemeManager::instance().setThemeMode(ThemeManager::ThemeMode::Dark);
    });

    QAction* systemAction = themeMenu->addAction(tr("&System"));
    systemAction->setCheckable(true);
    connect(systemAction, &QAction::triggered, this, [](){
        ThemeManager::instance().setThemeMode(ThemeManager::ThemeMode::System);
    });

    QActionGroup* themeGroup = new QActionGroup(this);
    themeGroup->addAction(lightAction);
    themeGroup->addAction(darkAction);
    themeGroup->addAction(systemAction);
    
    // Set initial state
    auto currentMode = ThemeManager::instance().themeMode();
    if (currentMode == ThemeManager::ThemeMode::Light) lightAction->setChecked(true);
    else if (currentMode == ThemeManager::ThemeMode::Dark) darkAction->setChecked(true);
    else systemAction->setChecked(true);

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

    setupNavigatorOverlayButton();

    // Create constraint panel (hidden initially)
    m_constraintPanel = new ConstraintPanel(m_viewport);
    m_constraintPanel->setVisible(false);

    m_viewport->installEventFilter(this);
}

void MainWindow::positionToolbarOverlay() {
    if (!m_viewport || !m_toolbar) {
        return;
    }

    const int margin = 20;
    const int xOffset = margin;
    const int availableHeight = m_viewport->height();
    const int toolbarHeight = m_toolbar->height();
    int yOffset = qMax(margin, (availableHeight - toolbarHeight) / 2);

    m_toolbar->move(xOffset, yOffset);
    m_toolbar->raise();
}

bool MainWindow::eventFilter(QObject* obj, QEvent* event) {
    if (obj == m_viewport && event->type() == QEvent::Resize) {
        positionToolbarOverlay();
        positionNavigatorOverlayButton();
        positionConstraintPanel();
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
    } else {
        updateDofStatus(nullptr);

        // Hide constraint panel
        if (m_constraintPanel) {
            m_constraintPanel->setVisible(false);
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
