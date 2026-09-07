import Quickshell
import QtQuick

// Every top-level component is loaded through a Loader rather than
// instantiated directly. A user can shadow any of these, and a broken
// override must cost them that one piece - not the whole shell. Without
// this, one stale Bar.qml takes the desktop down with it.
ShellRoot {
    component Piece: Loader {
        required property string name
        source: name + ".qml"
        onStatusChanged: {
            if (status === Loader.Error)
                console.warn("shell: " + name + " failed to load - skipping it");
        }
    }

    // One per screen, like the bar. A Loader for the same reason: a broken
    // Wallpaper.qml should cost you the wallpaper, not the desktop.
    Variants {
        model: Quickshell.screens
        Loader {
            required property var modelData
            Component.onCompleted: setSource("Wallpaper.qml", { modelData: modelData })
            onStatusChanged: {
                if (status === Loader.Error)
                    console.warn("shell: Wallpaper failed to load - no wallpaper this session");
            }
        }
    }

    Variants {
        model: Quickshell.screens
        Loader {
            required property var modelData
            // setSource, not source: Bar declares modelData as a required
            // property, and a required property has to be supplied at
            // creation. Assigning it in onLoaded is too late and the
            // component simply fails to build.
            Component.onCompleted: setSource("Bar.qml", { modelData: modelData })
            onStatusChanged: {
                if (status === Loader.Error)
                    console.warn("shell: Bar failed to load - no bar this session");
            }
        }
    }

    Piece { name: "Launcher" }
    Piece { name: "PowerMenu" }
    Piece { name: "Osd" }
    Piece { name: "Notifications" }
}
