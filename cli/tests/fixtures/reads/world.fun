type level = { name: string, height: float }
let load = (name): level =>
  let path = Path.of(name) in
  if File.exists(path) then { name: name, height: Bytes.length(File.bytes(path)) }
  else { name: "empty", height: 0.0 }
