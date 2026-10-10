.pragma library

// R4: songs whose credits came back empty in this session (CreditsDialog writes, TrackMenu
// reads when it opens). Plain JS state shared by every view; nothing binds to it.
var empty = ({})

function markEmpty(uri) { if (uri) empty[uri] = true }
function isEmpty(uri) { return empty[uri] === true }
