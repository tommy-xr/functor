// A file that defines `part` and none of these runs with them: the default
// part viewer. Paste them into the file to change any of them.
let init = PartViewer.init
let tick = PartViewer.tick
let draw = (m, tts) => PartViewer.draw(part, m, tts)
let mouseMove = PartViewer.mouseMove
let mouseButton = PartViewer.mouseButton
let mouseWheel = PartViewer.mouseWheel
