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

    /// The name in the state file, known before the directory has been listed.
    property string remembered: ""

    // The fallback is for a machine with no wallpapers, not for the second
    // before the list arrives. Listing the directory means running find, and
    // waiting for it showed the shipped default at every boot and then faded
    // it out - which looks like the default is one of your images.
    //
    // The state file already names the image that was showing, and the
    // directory is known, so the path can be built without waiting for
    // anything.
    readonly property string current:
        images.length > 0
            ? images[index % images.length]
            : remembered.length > 0
                ? conf.directory + "/" + remembered
                : conf.fallback

    // The one file that says which image is showing.
    //
    // Both this and `kiwami wallpaper set` write it, and both watch it, so
    // picking a wallpaper from a terminal is instant and rotation survives a
    // shell restart instead of snapping back to the first image. Two writers
    // are fine because there is only ever one value; what would not be fine
    // is two files.
    FileView {
        id: chosen
        path: root.conf.state || ""
        watchChanges: true
        atomicWrites: true
        printErrors: false
        // Read before the first frame rather than a moment after it: this is
        // ten bytes, and the whole point is to have it in time.
        preload: true
        blockLoading: true
        onFileChanged: reload()
        onLoaded: {
            const name = text().trim();
            if (name.length === 0) return;
            root.remembered = name;
            const at = root.images.findIndex(p => p.split("/").pop() === name);
            // Ignoring our own write, which would otherwise bounce straight
            // back and re-trigger the crossfade.
            if (at >= 0 && at !== root.index) root.index = at;
        }
    }

    function remember() {
        if (!conf.state || images.length === 0) return;
        const name = current.split("/").pop();
        if (chosen.text().trim() !== name) chosen.setText(name);
    }

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
    // Re-listed whenever the directory changes, which includes the moment it
    // first becomes known.
    //
    // Config starts on a fallback whose directory is the empty string and
    // loads the real manifest a frame later. The scan ran once, at creation,
    // against that empty string - so it listed nothing, and the next attempt
    // was the rotation timer. The shipped default sat on screen until then:
    // ten seconds here, fifteen minutes on the standard interval, looking
    // for all the world like the wallpaper had not been set.
    //
    // A binding updating the command of a process that has already exited
    // does not re-run it. Something has to say so.
    readonly property string dir: conf.directory || ""
    onDirChanged: rescan()

    /// Start a listing, with the command set rather than bound.
    ///
    /// It was bound to `dir`, and started from dir's own change handler.
    /// Nothing guarantees a dependent binding is re-evaluated before the
    /// handler runs, and it was not: the process launched with the previous
    /// command, `find ""`, which fails. The failure went to a stderr nobody
    /// was reading, the empty stdout looked like an empty directory, and the
    /// wallpaper stayed on the shipped default while every part of this
    /// reported success.
    function rescan() {
        if (dir.length === 0) return;
        scan.command = ["find", dir, "-maxdepth", "1", "-type", "f"];
        scan.running = true;
    }

    Process {
        id: scan
        running: false
        // A listing that fails should say so. Not reading this is what made
        // the bug above invisible for an afternoon.
        stderr: StdioCollector {
            onStreamFinished: {
                const err = (text || "").trim();
                if (err.length > 0) console.warn("wallpaper: listing failed:", err);
            }
        }

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
            root.rescan();
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
        remember();
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

            // A remembered name can be a file somebody has since deleted.
            // Without this the screen simply stays empty until the next
            // rotation, which reads as the wallpaper being broken.
            onStatusChanged: {
                if (status === Image.Error && layer.path !== root.conf.fallback)
                    layer.path = root.conf.fallback;
            }
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

    Component.onCompleted: {
        const name = chosen.text().trim();
        if (name.length > 0) remembered = name;
        a.path = current;
        rescan();
    }
}
