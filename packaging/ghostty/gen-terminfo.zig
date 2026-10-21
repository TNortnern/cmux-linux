// Prints ghostty's terminfo source (xterm-ghostty) to stdout.
//
// The libghostty build does not install ghostty's resources (zig build only does
// that for the full app), so packaging compiles this generator against the
// submodule's src/terminfo and feeds the output to `tic`. Mirrors
// `ghostty +terminfo` in ghostty's src/build/GhosttyResources.zig.
//
// Built by packaging/scripts/stage-ghostty-resources.sh.
const std = @import("std");
const terminfo = @import("terminfo");

pub fn main() !void {
    var buffer: [1024]u8 = undefined;
    var stdout_writer = std.fs.File.stdout().writer(&buffer);
    const writer = &stdout_writer.interface;
    try terminfo.ghostty.encode(writer);
    try stdout_writer.end();
}
