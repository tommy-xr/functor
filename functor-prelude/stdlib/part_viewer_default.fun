// A file that defines `part` and none of these runs with the default viewer.
let init = PartViewer.init
let tick = PartViewer.tick
let draw = (m, tts) => PartViewer.draw(part, m, tts)
let mouseMove = PartViewer.mouseMove
let mouseButton = PartViewer.mouseButton
let mouseWheel = PartViewer.mouseWheel
