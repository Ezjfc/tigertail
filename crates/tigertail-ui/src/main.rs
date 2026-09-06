//! On-device settings UI for tigertaild (QML via qmetaobject, Qt 5.15).
//!
//! Build with `--features qt` once the cross Qt toolchain is in the flake;
//! the default build is a stub so the workspace compiles everywhere. The
//! full plan and open display-stack questions (rm2fb) live in NOTES.md.

#[cfg(feature = "qt")]
fn main() {
    use qmetaobject::prelude::*;

    let mut engine = QmlEngine::new();
    engine.load_data(
        r#"
        import QtQuick 2.15
        import QtQuick.Window 2.15

        Window {
            visible: true
            width: 1404
            height: 1872
            color: "white"

            Text {
                anchors.centerIn: parent
                text: "tigertail"
                font.pixelSize: 96
            }
        }
        "#
        .into(),
    );
    engine.exec();
}

#[cfg(not(feature = "qt"))]
fn main() {
    eprintln!("tigertail-ui was built without the `qt` feature; this is a stub.");
    eprintln!("Rebuild with `cargo build -p tigertail-ui --features qt` in a Qt-enabled shell.");
    std::process::exit(1);
}
