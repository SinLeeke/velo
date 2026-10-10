import QtQuick
import "../library"
import "../../theme"

// Compatibility API for settings/device pickers; keyboard, focus, sizing and disabled state
// come from the same button as the rest of the library.
LibraryButton {
    property string kind: "default"
    primary: kind === "accent"
    restFill: Theme.surface2
    ink: !enabled ? Theme.textSecondary : primary ? Theme.accentInk
         : kind === "danger" ? Theme.danger : Theme.text
}
