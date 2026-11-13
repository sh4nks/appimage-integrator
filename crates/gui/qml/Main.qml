import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import AppImageIntegrator

ApplicationWindow {
    id: window

    readonly property var pending: backend.pending === "" ? null : JSON.parse(backend.pending)
    readonly property var apps: backend.apps === "" ? [] : JSON.parse(backend.apps)

    width: pending ? 560 : 760
    height: pending ? 280 : 520
    minimumWidth: 480
    minimumHeight: 240
    visible: true
    title: pending ? "Open " + pending.name : "AppImage Integrator"

    Backend {
        id: backend
        Component.onCompleted: start()
    }

    Loader {
        anchors.fill: parent
        sourceComponent: window.pending ? pendingPage : listPage
    }

    Component {
        id: pendingPage

        Item {
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: 20
                spacing: 16

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 16

                    Image {
                        Layout.preferredWidth: 64
                        Layout.preferredHeight: 64
                        source: window.pending.icon
                        sourceSize: Qt.size(128, 128)
                        fillMode: Image.PreserveAspectFit
                        visible: window.pending.icon !== ""
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2

                        Label {
                            Layout.fillWidth: true
                            text: window.pending.name
                            font.bold: true
                            font.pointSize: Application.font.pointSize * 1.4
                            elide: Text.ElideRight
                        }
                        Label {
                            text: "Version " + window.pending.version
                            visible: window.pending.version !== ""
                        }
                        Label {
                            Layout.fillWidth: true
                            text: window.pending.file
                            elide: Text.ElideMiddle
                            opacity: 0.7
                        }
                    }
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: window.pending.error !== ""
                        ? "This file cannot be integrated: " + window.pending.error + "."
                        : "This AppImage is not in your launcher yet. Integrating moves it to your applications folder and adds a launcher entry."
                }

                Label {
                    Layout.fillWidth: true
                    wrapMode: Text.WordWrap
                    text: backend.status
                    visible: backend.status !== ""
                }

                Item {
                    Layout.fillHeight: true
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    Item {
                        Layout.fillWidth: true
                    }
                    Button {
                        text: "Cancel"
                        onClicked: Qt.quit()
                    }
                    Button {
                        text: "Run once"
                        onClicked: if (backend.runPending()) Qt.quit()
                    }
                    Button {
                        text: "Integrate"
                        enabled: window.pending.error === ""
                        onClicked: backend.integratePending(false)
                    }
                    Button {
                        text: "Integrate and run"
                        highlighted: true
                        enabled: window.pending.error === ""
                        onClicked: if (backend.integratePending(true)) Qt.quit()
                    }
                }
            }
        }
    }

    Component {
        id: listPage

        ColumnLayout {
            spacing: 0

            ToolBar {
                Layout.fillWidth: true

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 4

                    Label {
                        text: window.apps.length === 1 ? "1 AppImage" : window.apps.length + " AppImages"
                    }
                    Item {
                        Layout.fillWidth: true
                    }
                    BusyIndicator {
                        Layout.preferredWidth: 24
                        Layout.preferredHeight: 24
                        running: backend.busy
                        visible: backend.busy
                    }
                    ToolButton {
                        text: "Check for updates"
                        visible: backend.updatable
                        enabled: !backend.busy && window.apps.length > 0
                        onClicked: backend.checkUpdates()
                    }
                    ToolButton {
                        text: "Refresh"
                        enabled: !backend.busy
                        onClicked: backend.refresh()
                    }
                }
            }

            ListView {
                id: list

                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: window.apps
                ScrollBar.vertical: ScrollBar {}

                delegate: Item {
                    id: row

                    required property var modelData

                    width: list.width
                    height: 76

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: 12
                        anchors.rightMargin: 12
                        spacing: 12

                        Image {
                            Layout.preferredWidth: 48
                            Layout.preferredHeight: 48
                            source: row.modelData.icon
                            sourceSize: Qt.size(96, 96)
                            fillMode: Image.PreserveAspectFit
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2

                            Label {
                                Layout.fillWidth: true
                                text: row.modelData.version === ""
                                    ? row.modelData.name
                                    : row.modelData.name + "  " + row.modelData.version
                                font.bold: true
                                elide: Text.ElideRight
                            }
                            Label {
                                Layout.fillWidth: true
                                text: row.modelData.missing ? "File is missing: " + row.modelData.path : row.modelData.path
                                elide: Text.ElideMiddle
                                opacity: 0.7
                            }
                            Label {
                                Layout.fillWidth: true
                                text: row.modelData.updateText
                                visible: row.modelData.updateText !== ""
                                elide: Text.ElideRight
                            }
                        }

                        Button {
                            text: "Update"
                            highlighted: true
                            visible: row.modelData.updateAvailable
                            enabled: !backend.busy
                            onClicked: backend.updateApp(row.modelData.id)
                        }
                        Button {
                            text: "Open"
                            enabled: !row.modelData.missing
                            onClicked: backend.launchApp(row.modelData.id)
                        }
                        Button {
                            text: "Remove"
                            enabled: !backend.busy
                            onClicked: removeDialog.ask(row.modelData)
                        }
                    }
                }

                Label {
                    anchors.centerIn: parent
                    width: parent.width - 80
                    visible: window.apps.length === 0
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    opacity: 0.7
                    text: "No AppImages are integrated yet. Open an AppImage with this app, or put one into a watched folder."
                }
            }

            Label {
                Layout.fillWidth: true
                padding: 8
                wrapMode: Text.WordWrap
                text: backend.status
                visible: backend.status !== ""
            }
        }
    }

    Dialog {
        id: removeDialog

        property var app: null

        function ask(target) {
            app = target
            deleteFile.checked = true
            open()
        }

        anchors.centerIn: parent
        modal: true
        title: app ? "Remove " + app.name + "?" : ""
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: backend.removeApp(app.id, !deleteFile.checked)

        ColumnLayout {
            spacing: 8

            Label {
                text: "This removes the launcher entry and icon."
            }
            CheckBox {
                id: deleteFile
                text: "Also delete the AppImage file"
                checked: true
            }
        }
    }
}
