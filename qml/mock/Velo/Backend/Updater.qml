pragma Singleton
import QtQuick

QtObject {
    property string state: "idle"
    property string latestVersion: ""
    property string currentVersion: "0.9.2"
    property real progress: 0
    property string error: ""
    property bool portable: false
    // Preview/test controls; production properties above are read-only.
    property int dismissCalls: 0
    property int installCalls: 0
    property int openReleasePageCalls: 0
    property int checkCalls: 0
    property string checkResult: "idle"
    property bool dismissed: false
    signal checkFinished(bool newer)
    function checkNow() {
        checkCalls++
        state = dismissed && checkResult === "available" ? "idle" : checkResult
        checkFinished(checkResult === "available")
    }
    function install() {
        installCalls++
        if (portable) openReleasePage()
        else { progress = 0; error = ""; state = "downloading" }
    }
    function dismiss() { dismissCalls++; dismissed = true; state = "idle" }
    function openReleasePage() { openReleasePageCalls++ }
}
