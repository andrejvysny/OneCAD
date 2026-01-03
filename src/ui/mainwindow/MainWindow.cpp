#include "MainWindow.h"
#include "../theme/ThemeManager.h"
#include "../viewport/Viewport.h"
#include "../navigator/ModelNavigator.h"
#include "../inspector/PropertyInspector.h"
#include "../toolbar/ContextToolbar.h"

#include <QMenuBar>
#include <QStatusBar>
#include <QLabel>
#include <QApplication>
#include <QMessageBox>
#include <QFileDialog>
#include <QActionGroup>

namespace onecad {
namespace ui {

MainWindow::MainWindow(QWidget* parent)
    : QMainWindow(parent) {
    
    setWindowTitle(tr("OneCAD"));
    resize(1280, 800);
    setMinimumSize(800, 600);
    
    applyTheme();
    setupMenuBar();
    setupToolBar();
    setupViewport();
    setupDocks();
    setupStatusBar();
}

MainWindow::~MainWindow() = default;

void MainWindow::applyTheme() {
    ThemeManager::instance().applyTheme();
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
    
    QAction* navAction = viewMenu->addAction(tr("&Navigator"));
    navAction->setCheckable(true);
    navAction->setChecked(true);
    connect(navAction, &QAction::toggled, this, [this](bool checked) {
        m_navigator->setVisible(checked);
    });
    
    QAction* inspAction = viewMenu->addAction(tr("&Inspector"));
    inspAction->setCheckable(true);
    inspAction->setChecked(true);
    connect(inspAction, &QAction::toggled, this, [this](bool checked) {
        m_inspector->setVisible(checked);
    });
    
    // Help menu
    QMenu* helpMenu = menuBar->addMenu(tr("&Help"));
    helpMenu->addAction(tr("&About OneCAD"), this, [this]() {
        QMessageBox::about(this, tr("About OneCAD"),
            tr("<h3>OneCAD</h3>"
               "<p>Version 0.1.0</p>"
               "<p>A beginner-friendly 3D CAD for makers.</p>"
               "<p>Built with Qt 6 + OpenCASCADE + Eigen3</p>"));
    });
}

void MainWindow::setupToolBar() {
    m_toolbar = new ContextToolbar(this);
    addToolBar(Qt::TopToolBarArea, m_toolbar);
    
    connect(m_toolbar, &ContextToolbar::newSketchRequested,
            this, &MainWindow::onNewSketch);
    connect(m_toolbar, &ContextToolbar::importRequested,
            this, &MainWindow::onImport);
}

void MainWindow::setupViewport() {
    m_viewport = new Viewport(this);
    setCentralWidget(m_viewport);
    
    connect(m_viewport, &Viewport::mousePositionChanged,
            this, &MainWindow::onMousePositionChanged);
}

void MainWindow::setupDocks() {
    // Left dock: Navigator
    m_navigator = new ModelNavigator(this);
    addDockWidget(Qt::LeftDockWidgetArea, m_navigator);
    
    // Right dock: Inspector
    m_inspector = new PropertyInspector(this);
    addDockWidget(Qt::RightDockWidgetArea, m_inspector);
    
    // Set corners to dock areas
    setCorner(Qt::TopLeftCorner, Qt::LeftDockWidgetArea);
    setCorner(Qt::BottomLeftCorner, Qt::LeftDockWidgetArea);
    setCorner(Qt::TopRightCorner, Qt::RightDockWidgetArea);
    setCorner(Qt::BottomRightCorner, Qt::RightDockWidgetArea);
}

void MainWindow::setupStatusBar() {
    QStatusBar* status = statusBar();
    
    m_toolStatus = new QLabel(tr("Ready"));
    m_toolStatus->setMinimumWidth(150);
    status->addWidget(m_toolStatus);
    
    m_dofStatus = new QLabel(tr("DOF: —"));
    m_dofStatus->setMinimumWidth(80);
    status->addWidget(m_dofStatus);
    
    m_coordStatus = new QLabel(tr("X: 0.00  Y: 0.00  Z: 0.00"));
    m_coordStatus->setMinimumWidth(200);
    status->addPermanentWidget(m_coordStatus);
}

void MainWindow::onNewSketch() {
    m_toolStatus->setText(tr("Select plane for new sketch..."));
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

} // namespace ui
} // namespace onecad
