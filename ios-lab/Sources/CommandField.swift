import SwiftUI
import UIKit

/// The command line. A UITextField, not a SwiftUI TextField: smart quotes
/// and dashes must be off ("--" would become "—", which the shell can't
/// read), and Return must send without dropping the keyboard.
struct CommandField: UIViewRepresentable {
    @Binding var text: String
    var onSubmit: () -> Void

    func makeUIView(context: Context) -> UITextField {
        let field = UITextField()
        field.font = .monospacedSystemFont(ofSize: 15, weight: .regular)
        field.textColor = .white
        field.tintColor = UIColor(Tone.ok.color)
        field.autocapitalizationType = .none
        field.autocorrectionType = .no
        field.spellCheckingType = .no
        field.smartQuotesType = .no
        field.smartDashesType = .no
        field.smartInsertDeleteType = .no
        field.returnKeyType = .send
        field.keyboardAppearance = .dark
        field.clearButtonMode = .whileEditing
        field.attributedPlaceholder = NSAttributedString(
            string: "command",
            attributes: [.foregroundColor: UIColor(Tone.dim.color)]
        )
        field.delegate = context.coordinator
        field.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .editingChanged)
        // Fill the space the prompt leaves.
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    func updateUIView(_ field: UITextField, context: Context) {
        context.coordinator.parent = self
        if field.text != text { field.text = text }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    final class Coordinator: NSObject, UITextFieldDelegate {
        var parent: CommandField

        init(parent: CommandField) {
            self.parent = parent
        }

        @objc func changed(_ field: UITextField) {
            parent.text = field.text ?? ""
        }

        func textFieldShouldReturn(_ field: UITextField) -> Bool {
            parent.onSubmit()
            return false // keep the keyboard up for the next command
        }
    }
}
