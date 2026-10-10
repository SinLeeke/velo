import QtQuick
import "../../theme"

// M1b: "are you sure?" before a destructive action (Eliminar playlist). Never a native or JS
// dialog. Cancelar holds the focus when it opens, so Enter or Esc never destroy by accident.
LibraryDialog {
    id: dialog
    property string message: ""
    property string confirmText: qsTr("Eliminar")
    signal confirmed()

    initialFocus: cancel

    LibraryLabel {
        width: parent.width
        text: dialog.message
        color: Theme.textSecondary
        wrapMode: Text.Wrap
        lineHeight: Theme.type.bodyLineHeight
        maximumLineCount: 6
    }

    buttons: [
        LibraryButton { id: cancel; objectName: "confirmCancelButton"; text: qsTr("Cancelar"); onClicked: dialog.close() },
        LibraryButton {
            objectName: "confirmAcceptButton"
            primary: true
            text: dialog.confirmText
            onClicked: { dialog.close(); dialog.confirmed() }
        }
    ]
}
