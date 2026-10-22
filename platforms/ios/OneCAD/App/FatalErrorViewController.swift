import UIKit

/// Full-screen native error shown when the Rust app cannot start or has
/// entered a terminal state. Recovery requires relaunching the app.
final class FatalErrorViewController: UIViewController {
    enum Kind {
        case startupFailed
        case relaunchRequired

        var accessibilityID: String {
            switch self {
            case .startupFailed: "onecad.error.startup"
            case .relaunchRequired: "onecad.error.relaunchRequired"
            }
        }

        var title: String {
            switch self {
            case .startupFailed: "OneCAD Could Not Start"
            case .relaunchRequired: "OneCAD Must Be Relaunched"
            }
        }
    }

    private let kind: Kind
    private let message: String

    init(kind: Kind, message: String) {
        self.kind = kind
        self.message = message
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .systemBackground
        view.accessibilityIdentifier = kind.accessibilityID
        let title = label(kind.title, style: .title2, id: "onecad.error.title")
        let detail = label(message, style: .body, id: "onecad.error.message")
        let hint = label("Your last saved and autosaved work is kept. Close OneCAD from the app switcher and open it again.",
                         style: .footnote, id: "onecad.error.hint")
        let stack = UIStackView(arrangedSubviews: [title, detail, hint])
        stack.axis = .vertical
        stack.spacing = 12
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        let guide = view.readableContentGuide
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: guide.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: guide.trailingAnchor),
            stack.centerYAnchor.constraint(equalTo: view.centerYAnchor),
        ])
    }

    private func label(_ text: String, style: UIFont.TextStyle, id: String) -> UILabel {
        let label = UILabel()
        label.text = text
        label.font = .preferredFont(forTextStyle: style)
        label.adjustsFontForContentSizeCategory = true
        label.numberOfLines = 0
        label.textAlignment = .center
        label.accessibilityIdentifier = id
        return label
    }
}
