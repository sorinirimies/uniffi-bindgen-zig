//! Fuzzes the decoding of values coming from Rust, in the code generated for
//! the `tour` fixture: whatever the bytes, reading them either fails with
//! `error.Internal` (freeing whatever it had read) or yields a value that writes
//! back to exactly the bytes it was read from. `std.testing.allocator` fails the
//! test on a leak, so every `errdefer` path is checked too.
//!
//! Run by scripts/ci/fuzz_zig.nu; UNIFFI_FUZZ_RUNS sets the number of inputs.
const std = @import("std");
const tour = @import("tour.zig");
const uniffi = @import("uniffi.zig");

const gpa = std.testing.allocator;

fn roundTrip(comptime T: type, comptime read: anytype, comptime write: anytype, input: []const u8) !void {
    var r: uniffi.Reader = .{ .data = input };
    const value: T = read(&r, gpa) catch |e| switch (e) {
        error.Internal => return,
        else => return e,
    };
    defer if (@hasDecl(T, "deinit")) value.deinit(gpa);
    var w: uniffi.Writer = .{};
    defer w.deinit();
    write(&w, value);
    if (!std.mem.eql(u8, w.list.items, input[0..r.pos])) {
        std.debug.print("round trip changed the bytes of {s}\n", .{@typeName(T)});
        return error.TestUnexpectedResult;
    }
}

fn readString(r: *uniffi.Reader, a: std.mem.Allocator) uniffi.Error![]const u8 {
    return r.string(a);
}

fn writeString(w: *uniffi.Writer, v: []const u8) void {
    w.string(v);
}

fn one(data: []const u8) !void {
    if (data.len == 0) return;
    const rest = data[1..];
    switch (data[0] % 6) {
        0 => {
            var r: uniffi.Reader = .{ .data = rest };
            const s = readString(&r, gpa) catch return;
            defer gpa.free(s);
            var w: uniffi.Writer = .{};
            defer w.deinit();
            writeString(&w, s);
            try std.testing.expectEqualSlices(u8, rest[0..r.pos], w.list.items);
        },
        1 => try roundTrip(tour.Settings, tour.uniffiReadSettings, tour.uniffiWriteSettings, rest),
        2 => try roundTrip(tour.Shape, tour.uniffiReadShape, tour.uniffiWriteShape, rest),
        3 => try roundTrip(tour.Expr, tour.uniffiReadExpr, tour.uniffiWriteExpr, rest),
        4 => try roundTrip(tour.TreeNode, tour.uniffiReadTreeNode, tour.uniffiWriteTreeNode, rest),
        else => {
            var r: uniffi.Reader = .{ .data = rest };
            const ts = r.timestamp() catch return;
            var w: uniffi.Writer = .{};
            defer w.deinit();
            w.timestamp(ts);
            try std.testing.expectEqualSlices(u8, rest[0..r.pos], w.list.items);
        },
    }
}

test "decoding arbitrary bytes fails cleanly or round-trips" {
    const runs: usize = 200_000;
    var prng = std.Random.DefaultPrng.init(42);
    const random = prng.random();
    var input: [64]u8 = undefined;
    for (0..runs) |_| {
        const len = random.uintLessThan(usize, input.len);
        for (input[0..len]) |*b| {
            // Mostly small values, so lengths and tags are often plausible.
            b.* = if (random.uintLessThan(u8, 8) == 0) random.int(u8) else random.uintLessThan(u8, 4);
        }
        try one(input[0..len]);
    }
}
