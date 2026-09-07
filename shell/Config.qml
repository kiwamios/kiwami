pragma Singleton
import Quickshell
import Quickshell.Io
import QtQuick

// Layout comes from Nix, via /etc/kiwami/bar.json. Setting kiwami.bar.right in
// a consumer's flake moves things here without touching any QML.
Singleton {
    id: root

    readonly property var fallback: ({
        enable: true, position: "top", height: 32,
        left: ["workspaces"], center: ["window"],
        right: ["tray", "battery", "clock"]
    })

    property var bar: fallback

    readonly property var wallpaperFallback: ({
        enable: true, directory: "", rotate: false, interval: 900,
        fit: "cover", fallback: "/etc/kiwami/wallpaper-default.svg"
    })

    property var wallpaper: wallpaperFallback

    FileView {
        id: paper
        path: "/etc/kiwami/wallpaper.json"
        watchChanges: true
        onFileChanged: reload()
        onLoadFailed: root.wallpaper = root.wallpaperFallback
        onLoaded: {
            try {
                root.wallpaper = JSON.parse(paper.text());
            } catch (e) {
                console.warn("wallpaper.json unreadable, using defaults:", e);
                root.wallpaper = root.wallpaperFallback;
            }
        }
    }

    FileView {
        id: file
        path: "/etc/kiwami/bar.json"
        watchChanges: true
        onFileChanged: reload()
        onLoadFailed: root.bar = root.fallback
        onLoaded: {
            try {
                root.bar = JSON.parse(file.text());
            } catch (e) {
                console.warn("bar.json unreadable, using defaults:", e);
                root.bar = root.fallback;
            }
        }
    }
}
