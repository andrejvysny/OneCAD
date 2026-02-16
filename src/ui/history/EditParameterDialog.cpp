/**
 * @file EditParameterDialog.cpp
 * @brief Implementation of EditParameterDialog.
 */
#include "EditParameterDialog.h"
#include "../../app/commands/CommandProcessor.h"
#include "../../app/commands/UpdateOperationParamsCommand.h"
#include "../../app/document/Document.h"
#include "../../app/document/OperationRecord.h"
#include "../../app/history/RegenerationEngine.h"
#include "../../core/sketch/Sketch.h"
#include "../../render/tessellation/TessellationCache.h"
#include "../viewport/Viewport.h"

#include <QDoubleSpinBox>
#include <QLabel>
#include <QVBoxLayout>
#include <QHBoxLayout>
#include <QFormLayout>
#include <QPushButton>
#include <QDialogButtonBox>
#include <QLoggingCategory>

namespace onecad::ui {

Q_LOGGING_CATEGORY(logEditParamsDialog, "onecad.ui.history.editparams")

namespace {
constexpr int kDebounceMs = 100;
constexpr double kMinDistance = -10000.0;
constexpr double kMaxDistance = 10000.0;
constexpr double kMinAngle = -360.0;
constexpr double kMaxAngle = 360.0;
constexpr double kMinDraft = -89.0;
constexpr double kMaxDraft = 89.0;

std::unique_ptr<app::Document> makePreviewDocument(const app::Document& source) {
    auto preview = std::make_unique<app::Document>();

    preview->elementMap().fromString(source.elementMap().toString());

    for (const auto& sketchId : source.getSketchIds()) {
        const auto* sketch = source.getSketch(sketchId);
        if (!sketch) {
            continue;
        }
        auto clone = core::sketch::Sketch::fromJson(sketch->toJson());
        if (!clone) {
            continue;
        }
        preview->addSketchWithId(sketchId, std::move(clone), source.getSketchName(sketchId));
    }

    for (const auto& bodyId : source.getBodyIds()) {
        const TopoDS_Shape* shape = source.getBodyShape(bodyId);
        if (!shape || shape->IsNull()) {
            continue;
        }
        preview->addBodyWithId(bodyId, *shape, source.getBodyName(bodyId));
    }

    for (const auto& op : source.operations()) {
        preview->addOperation(op);
    }

    preview->setOperationSuppressionState(source.operationSuppressionState());
    preview->setBaseBodyIds(source.baseBodyIds());
    return preview;
}
} // namespace

EditParameterDialog::EditParameterDialog(app::Document* document,
                                         Viewport* viewport,
                                         app::commands::CommandProcessor* commandProcessor,
                                         const std::string& opId,
                                         QWidget* parent)
    : QDialog(parent)
    , document_(document)
    , viewport_(viewport)
    , commandProcessor_(commandProcessor)
    , opId_(opId) {
    setWindowTitle(tr("Edit Parameters"));
    setModal(true);
    setMinimumWidth(300);

    // Setup debounce timer
    debounceTimer_ = new QTimer(this);
    debounceTimer_->setSingleShot(true);
    debounceTimer_->setInterval(kDebounceMs);
    connect(debounceTimer_, &QTimer::timeout, this, &EditParameterDialog::updatePreview);

    setupUi();
    loadCurrentParams();
}

EditParameterDialog::~EditParameterDialog() {
    clearPreview();
}

void EditParameterDialog::setupUi() {
    auto* mainLayout = new QVBoxLayout(this);

    // Title
    auto* titleLabel = new QLabel(tr("Edit Operation Parameters"));
    titleLabel->setStyleSheet("font-weight: bold; font-size: 14px;");
    mainLayout->addWidget(titleLabel);

    // Parameters form
    paramsLayout_ = new QVBoxLayout;
    mainLayout->addLayout(paramsLayout_);

    mainLayout->addStretch();

    // Buttons
    auto* buttonBox = new QDialogButtonBox(
        QDialogButtonBox::Ok | QDialogButtonBox::Cancel);
    connect(buttonBox, &QDialogButtonBox::accepted, this, &QDialog::accept);
    connect(buttonBox, &QDialogButtonBox::rejected, this, &QDialog::reject);
    mainLayout->addWidget(buttonBox);
}

void EditParameterDialog::loadCurrentParams() {
    if (!document_) return;

    // Find the operation
    for (const auto& op : document_->operations()) {
        if (op.opId == opId_) {
            if (op.type == app::OperationType::Extrude) {
                isExtrude_ = true;
                if (std::holds_alternative<app::ExtrudeParams>(op.params)) {
                    buildExtrudeUi(std::get<app::ExtrudeParams>(op.params));
                }
            } else if (op.type == app::OperationType::Revolve) {
                isExtrude_ = false;
                if (std::holds_alternative<app::RevolveParams>(op.params)) {
                    buildRevolveUi(std::get<app::RevolveParams>(op.params));
                }
            }
            break;
        }
    }
}

void EditParameterDialog::buildExtrudeUi(const app::ExtrudeParams& params) {
    auto* formLayout = new QFormLayout;

    // Distance spinbox
    distanceSpinbox_ = new QDoubleSpinBox;
    distanceSpinbox_->setRange(kMinDistance, kMaxDistance);
    distanceSpinbox_->setValue(params.distance);
    distanceSpinbox_->setSuffix(" mm");
    distanceSpinbox_->setDecimals(2);
    distanceSpinbox_->setSingleStep(1.0);
    connect(distanceSpinbox_, QOverload<double>::of(&QDoubleSpinBox::valueChanged),
            this, &EditParameterDialog::onValueChanged);
    formLayout->addRow(tr("Distance:"), distanceSpinbox_);

    // Draft angle spinbox
    draftAngleSpinbox_ = new QDoubleSpinBox;
    draftAngleSpinbox_->setRange(kMinDraft, kMaxDraft);
    draftAngleSpinbox_->setValue(params.draftAngleDeg);
    draftAngleSpinbox_->setSuffix("°");
    draftAngleSpinbox_->setDecimals(1);
    draftAngleSpinbox_->setSingleStep(1.0);
    connect(draftAngleSpinbox_, QOverload<double>::of(&QDoubleSpinBox::valueChanged),
            this, &EditParameterDialog::onValueChanged);
    formLayout->addRow(tr("Draft Angle:"), draftAngleSpinbox_);

    paramsLayout_->addLayout(formLayout);
}

void EditParameterDialog::buildRevolveUi(const app::RevolveParams& params) {
    auto* formLayout = new QFormLayout;

    // Angle spinbox
    angleSpinbox_ = new QDoubleSpinBox;
    angleSpinbox_->setRange(kMinAngle, kMaxAngle);
    angleSpinbox_->setValue(params.angleDeg);
    angleSpinbox_->setSuffix("°");
    angleSpinbox_->setDecimals(1);
    angleSpinbox_->setSingleStep(15.0);
    connect(angleSpinbox_, QOverload<double>::of(&QDoubleSpinBox::valueChanged),
            this, &EditParameterDialog::onValueChanged);
    formLayout->addRow(tr("Angle:"), angleSpinbox_);

    paramsLayout_->addLayout(formLayout);
}

app::ExtrudeParams EditParameterDialog::getExtrudeParams() const {
    app::ExtrudeParams params;
    params.distance = distanceSpinbox_ ? distanceSpinbox_->value() : 0.0;
    params.draftAngleDeg = draftAngleSpinbox_ ? draftAngleSpinbox_->value() : 0.0;
    params.booleanMode = app::BooleanMode::NewBody;  // Preserve original mode
    params.targetBodyId.clear();

    // Find original to preserve boolean mode and target body
    if (document_) {
        for (const auto& op : document_->operations()) {
            if (op.opId == opId_ && std::holds_alternative<app::ExtrudeParams>(op.params)) {
                const auto& orig = std::get<app::ExtrudeParams>(op.params);
                params.booleanMode = orig.booleanMode;
                params.targetBodyId = orig.targetBodyId;
                qCDebug(logEditParamsDialog) << "getExtrudeParams:preserved-target"
                                             << QString::fromStdString(params.targetBodyId);
                break;
            }
        }
    }
    return params;
}

app::RevolveParams EditParameterDialog::getRevolveParams() const {
    app::RevolveParams params;
    params.angleDeg = angleSpinbox_ ? angleSpinbox_->value() : 360.0;
    params.booleanMode = app::BooleanMode::NewBody;
    params.targetBodyId.clear();

    // Find original to preserve boolean mode, axis, and target body
    if (document_) {
        for (const auto& op : document_->operations()) {
            if (op.opId == opId_ && std::holds_alternative<app::RevolveParams>(op.params)) {
                const auto& orig = std::get<app::RevolveParams>(op.params);
                params.booleanMode = orig.booleanMode;
                params.axis = orig.axis;
                params.targetBodyId = orig.targetBodyId;
                qCDebug(logEditParamsDialog) << "getRevolveParams:preserved-target"
                                             << QString::fromStdString(params.targetBodyId);
                break;
            }
        }
    }
    return params;
}

void EditParameterDialog::onValueChanged() {
    hasChanges_ = true;
    debounceTimer_->start();
}

void EditParameterDialog::updatePreview() {
    if (!document_ || !viewport_) return;
    qCDebug(logEditParamsDialog) << "updatePreview:start"
                                 << "opId=" << QString::fromStdString(opId_);

    // Create temporary params variant
    app::OperationParams newParams;
    if (isExtrude_) {
        newParams = getExtrudeParams();
    } else {
        newParams = getRevolveParams();
    }

    auto previewDoc = makePreviewDocument(*document_);
    auto* op = previewDoc->findOperation(opId_);
    if (!op) {
        qCWarning(logEditParamsDialog) << "updatePreview:operation-not-found"
                                       << QString::fromStdString(opId_);
        return;
    }
    op->params = newParams;

    app::history::RegenerationEngine engine(previewDoc.get());
    auto result = engine.regenerateAll();
    if (result.status == app::history::RegenStatus::CriticalFailure) {
        qCWarning(logEditParamsDialog) << "updatePreview:critical-regeneration-failure"
                                       << "opId=" << QString::fromStdString(opId_);
        clearPreview();
        return;
    }

    render::TessellationCache tessellator;
    std::vector<render::SceneMeshStore::Mesh> meshes;
    for (const auto& bodyId : previewDoc->getBodyIds()) {
        const TopoDS_Shape* shape = previewDoc->getBodyShape(bodyId);
        if (!shape || shape->IsNull()) {
            continue;
        }
        meshes.push_back(tessellator.buildMesh(bodyId, *shape, previewDoc->elementMap()));
    }
    viewport_->setModelPreviewMeshes(meshes);
    qCDebug(logEditParamsDialog) << "updatePreview:done"
                                 << "opId=" << QString::fromStdString(opId_)
                                 << "meshCount=" << meshes.size();

    emit previewRequested(QString::fromStdString(opId_));
}

void EditParameterDialog::clearPreview() {
    if (viewport_) {
        viewport_->clearModelPreviewMeshes();
    }
}

void EditParameterDialog::accept() {
    if (hasChanges_) {
        applyChanges();
    }
    clearPreview();
    QDialog::accept();
}

void EditParameterDialog::reject() {
    clearPreview();
    QDialog::reject();
}

void EditParameterDialog::applyChanges() {
    if (!document_) return;

    // Update operation params
    app::OperationParams newParams;
    if (isExtrude_) {
        newParams = getExtrudeParams();
    } else {
        newParams = getRevolveParams();
    }

    auto command = std::make_unique<app::commands::UpdateOperationParamsCommand>(
        document_, opId_, newParams);
    if (commandProcessor_) {
        commandProcessor_->execute(std::move(command));
    } else {
        command->execute();
    }

    emit parametersChanged(QString::fromStdString(opId_));
}

} // namespace onecad::ui
