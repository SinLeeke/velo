pragma Singleton
import QtQuick

// MOCK of Velo.Backend.Account.
QtObject {
    property bool signedIn: true
    property string displayName: "Juan"
    property url avatarUrl: ""
    property bool isPremium: true
    // "" = cuenta de Spotify (un solo login, L1); un ID = app personal.
    property string clientId: ""
    property string appWarning: ""
    property bool busy: false
    property bool audioReady: false
    property bool audioBusy: false

    signal signInFailed(string reason)
    signal audioFailed(string reason)

    function signIn() { if (clientId === "") audioReady = true; signedIn = true }
    function cancelSignIn() { busy = false; signInFailed(qsTr("Cancelaste el inicio de sesión. Puedes volver a intentarlo.")) }
    function signOut() { busy = false; audioReady = false; audioBusy = false; displayName = ""; avatarUrl = ""; isPremium = false; appWarning = ""; signedIn = false }
    function setClientId(id) { clientId = id }
    function authorizeClient(id) { clientId = id; appWarning = ""; signedIn = true; busy = false }
    function useSpotifyAccount() { clientId = ""; appWarning = ""; audioReady = true; signedIn = true; busy = false }
    function enableAudio() { audioReady = true }
}
