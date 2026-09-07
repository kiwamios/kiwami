import Quickshell
import Quickshell.Wayland
import Quickshell.Io
import Quickshell.Services.UPower
import QtQuick

// The wallpaper, as a layer-shell surface on the background layer - which is
// exactly what a wallpaper daemon is. Doing it here costs no extra process and
// no extra dependency, and the pictures get to be reactive: they can respond
// to the battery, which is the one thing that matters below.
//
// Policy comes from Nix (Config.wallpaper). Content is a directory of files
// the user owns. The two are deliberately not the same thing: changing a
// picture must never mean a rebuild.
PanelWindow {
    id: root
    required property var modelData
    screen: modelData

    readonly property var conf: Config.wallpaper

    // Behind everything, and claiming no space: a wallpaper that reserved an
    // exclusion zone would push every other window off the screen.
    WlrLayershell.layer: WlrLayer.Background
    WlrLayershell.namespace: "kiwami-wallpaper"
    exclusionMode: ExclusionMode.Ignore

    anchors { top: true; bottom: true; left: true; right: true }
    color: Theme.bg
    visible: conf.enable

    property var images: []
    property int index: 0

    readonly property string current:
        images.length > 0 ? images[index % images.length] : conf.fallback

    function fillMode() {
        switch (conf.fit) {
        case "contain": return Image.PreserveAspectFit;
        case "fill":    return Image.Stretch;
        case "tile":    return Image.Tile;
        default:        return Image.PreserveAspectCrop;
        }
    }

    // Quickshell has no directory model - Quickshell.Io offers FileView,
    // Process, Socket and streams, and Qt.labs.folderlistmodel is not in the
    // QML path - so the listing is a subprocess. Which is no loss: it means
    // the sort and the filter are visible here rather than implied.
    Process {
        id: scan
        running: true
        command: ["find", root.conf.directory, "-maxdepth", "1", "-type", "f"]
        stdout: StdioCollector {
            onStreamFinished: {
                const found = text.split("\n")
                    .map(l => l.trim())
                    .filter(l => l.length > 0 && /\.(png|jpe?g|svg|gif)$/i.test(l))
                    // Sorted, because directory order is whatever the
                    // filesystem feels like and "the first image" has to mean
                    // the same thing on a laptop and in a test VM.
                    .sort();

                // Rotation restarts rather than jumping: if the picture you
                // were on is still there, stay on it.
                const was = root.current;
                root.images = found;
                const still = found.indexOf(was);
                if (still >= 0) root.index = still;
            }
        }
    }

    // A directory that gains a photo should show it without restarting the
    // shell. Cheap: one `find` on the interval we are already waking up for.
    Timer {
        interval: root.conf.interval * 1000
        running: root.visible
        repeat: true
        onTriggered: {
            scan.running = true;
            if (root.conf.rotate && root.images.length > 1)
                root.index = (root.index + 1) % root.images.length;
        }
    }

    // Two layers so a change is a crossfade rather than a blink. The hidden
    // one loads the next picture, then they swap.
    property bool showA: true
    onCurrentChanged: {
        if (showA) b.path = current; else a.path = current;
        showA = !showA;
    }

    component Layer: Item {
        id: layer
        anchors.fill: parent
        property string path: ""

        // .gif gets QMovie via AnimatedImage; everything else is a plain
        // Image. Chosen by extension rather than by trying both, because a
        // failed decode is silent and you would only notice the wallpaper
        // missing.
        readonly property bool animated: /\.gif$/i.test(path)

        Image {
            anchors.fill: parent
            visible: !layer.animated
            source: layer.animated ? "" : (layer.path ? "file://" + layer.path : "")
            fillMode: root.fillMode()
            asynchronous: true
            cache: false
        }

        AnimatedImage {
            anchors.fill: parent
            visible: layer.animated
            source: layer.animated ? "file://" + layer.path : ""
            fillMode: root.fillMode()
            cache: false

            // The one piece of special treatment an animated wallpaper needs,
            // and it is not about decoding.
            //
            // An animated wallpaper repaints forever - including while it is
            // completely covered by windows, where not one of those frames can
            // be seen. On a laptop that is measurable battery spent drawing
            // pixels for nobody. Plugged in it is free, so it plays; on
            // battery it holds a frame, which is the choice you would make by
            // hand every time.
            playing: !UPower.onBattery
        }
    }

    Layer { id: a; opacity: root.showA ? 1 : 0; Behavior on opacity { NumberAnimation { duration: 600 } } }
    Layer { id: b; opacity: root.showA ? 0 : 1; Behavior on opacity { NumberAnimation { duration: 600 } } }

    Component.onCompleted: a.path = current
}
