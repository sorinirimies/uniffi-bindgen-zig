//! End-to-end test of the generated Zig bindings against the `tour` fixture.
//! `std.testing.allocator` fails the test on any leak.
const std = @import("std");
const tour = @import("tour.zig");
const uniffi = @import("uniffi.zig");

const t = std.testing;
const gpa = t.allocator;

test "primitives" {
    try t.expectEqual(@as(u8, 44), try tour.addU8(200, 100));
    try t.expectEqual(@as(i32, -2), try tour.addI32(-5, 3));
    try t.expectEqual((@as(u64, 1) << 40) + 5, try tour.addU64(1 << 40, 5));
    try t.expectEqual(@as(i64, -9), try tour.negateI64(9));
    try t.expectEqual(@as(f32, 1.5), try tour.halfF32(3.0));
    try t.expectEqual(@as(f64, 2.5), try tour.halfF64(5.0));
    try t.expect(!try tour.invert(true));
    const loud = try tour.shout(gpa, "héllo");
    defer gpa.free(loud);
    try t.expectEqualStrings("HÉLLO", loud);
    const reversed = try tour.reverseBytes(gpa, &.{ 1, 2, 3 });
    defer gpa.free(reversed);
    try t.expectEqualSlices(u8, &.{ 3, 2, 1 }, reversed);
    try tour.nothing();
    const hello = try tour.greet(gpa, "Ada", "Hello");
    defer gpa.free(hello);
    try t.expectEqualStrings("Hello, Ada!", hello);
}

test "containers" {
    try t.expectEqual(@as(?i32, 8), try tour.maybeDouble(4));
    try t.expectEqual(@as(?i32, null), try tour.maybeDouble(null));
    try t.expectEqual(@as(i64, 10), try tour.sum(&.{ 1, 2, 3, 4 }));
    try t.expectEqual(@as(i64, 0), try tour.sum(&.{}));

    const lengths = try tour.wordLengths(gpa, &.{ "a", "bcd", "ef" });
    defer {
        for (lengths) |e| gpa.free(e.key);
        gpa.free(lengths);
    }
    try t.expectEqual(@as(usize, 3), lengths.len);
    var found = false;
    for (lengths) |e| {
        if (std.mem.eql(u8, e.key, "bcd")) found = e.value == 3;
    }
    try t.expect(found);

    const nested = try tour.nested(gpa, &.{ null, &.{ "x", "y" }, &.{} });
    defer {
        for (nested) |item| if (item) |list| {
            for (list) |s| gpa.free(s);
            gpa.free(list);
        };
        gpa.free(nested);
    }
    try t.expectEqual(@as(usize, 3), nested.len);
    try t.expect(nested[0] == null);
    try t.expectEqualStrings("y", nested[1].?[1]);
    try t.expectEqual(@as(usize, 0), nested[2].?.len);

    const ts = try tour.timestampPlus(.{ .secs = 100 }, .{ .secs = 2, .nanos = 500_000_000 });
    try t.expectEqual(uniffi.Timestamp{ .secs = 102, .nanos = 500_000_000 }, ts);
    const before = try tour.timestampPlus(.{ .secs = -10 }, .{ .secs = 4, .nanos = 250_000_000 });
    try t.expectEqual(uniffi.Timestamp{ .secs = -6, .nanos = 250_000_000 }, before);
    try t.expectEqual(uniffi.Duration{ .secs = 3 }, try tour.doubleDuration(.{ .secs = 1, .nanos = 500_000_000 }));
}

test "records and enums" {
    try t.expectEqual(tour.Color.green, try tour.mix(.red));
    try t.expectEqual(tour.Color.red, try tour.mix(.blue));

    const circle = try tour.makeShape(0);
    try t.expectEqual(@as(f64, 2.0), circle.circle.radius);
    try t.expectEqual(@as(f64, 2.0), circle.circle.center.y);
    const rect = try tour.makeShape(1);
    try t.expectEqual(@as(f64, 2.0), rect.rect.v1.x);
    try t.expect(try tour.makeShape(2) == .empty);
    try t.expectEqual(@as(f64, 12.0), try tour.area(circle));
    try t.expectEqual(@as(f64, 6.0), try tour.area(rect));
    try t.expectEqual(@as(f64, 3.0), try tour.area(.{ .circle = .{ .center = .{ .x = 0, .y = 0 }, .radius = 1.0 } }));

    // Default field values.
    const s: tour.Settings = .{ .name = "n", .origin = .{ .x = 7, .y = 8 } };
    const d = try tour.describe(gpa, s);
    defer gpa.free(d);
    try t.expectEqualStrings("n:8:None::7", d);
    const full: tour.Settings = .{ .name = "n", .retries = 2, .label = "L", .tags = &.{ "a", "b" }, .origin = .{ .x = 7, .y = 8 } };
    const d2 = try tour.describe(gpa, full);
    defer gpa.free(d2);
    try t.expectEqualStrings("n:2:Some(\"L\"):a+b:7", d2);
}

test "errors" {
    try tour.failData(0, null);
    var err: tour.DataError = undefined;
    try t.expectError(error.BadCode, tour.failData(1, &err));
    try t.expectEqual(@as(i32, 42), err.bad_code.code);
    try t.expectError(error.TooLong, tour.failData(2, &err));
    try t.expectEqual(@as(u32, 9), err.too_long.v0);
    try t.expectError(error.Plain, tour.failData(3, null));

    var ferr: tour.FlatError = undefined;
    try t.expectEqual(@as(u32, 7), try tour.failFlat(gpa, 0, &ferr));
    try t.expectError(error.NotFound, tour.failFlat(gpa, 1, &ferr));
    try t.expectError(error.Denied, tour.failFlat(gpa, 2, &ferr));
    defer ferr.deinit(gpa);
    try t.expectEqualStrings("nope", ferr.denied.v0);
    // Without an err out-parameter the payload is freed by the bindings.
    try t.expectError(error.Denied, tour.failFlat(gpa, 2, null));

    var merr: tour.MessageError = undefined;
    try t.expectError(error.Io, tour.failMessage(gpa, 1, &merr));
    defer merr.deinit(gpa);
    try t.expectEqualStrings("io failure: disk", merr.io);

    try t.expectError(error.RustPanic, tour.panics());
    try t.expect(std.mem.indexOf(u8, uniffi.lastErrorMessage(), "boom") != null);
}

test "objects" {
    const counter = try tour.Counter.init(10);
    defer counter.deinit();
    try t.expectEqual(@as(i64, 10), try counter.get());
    try t.expectEqual(@as(i64, 15), try counter.add(5));
    const zero = try tour.Counter.zero();
    defer zero.deinit();
    try t.expectEqual(@as(i64, 0), try zero.get());
    var ferr: tour.FlatError = undefined;
    try t.expectError(error.Denied, tour.Counter.checked(gpa, -1, &ferr));
    ferr.deinit(gpa);
    const three = try tour.Counter.checked(gpa, 3, null);
    defer three.deinit();
    try t.expectEqual(@as(i64, 3), try three.get());

    const other = try tour.Counter.init(100);
    defer other.deinit();
    try counter.merge(other);
    try t.expectEqual(@as(i64, 115), try counter.get());
    try t.expectEqual(@as(i64, 100), try other.get());

    const twin = try counter.twin();
    defer twin.deinit();
    try t.expect(twin.handle != counter.handle);
    try t.expect(try twin.eql(counter));
    const maybe = try counter.maybeTwin(true);
    try t.expect(maybe != null);
    maybe.?.deinit();
    try t.expect(try counter.maybeTwin(false) == null);
    const text = try counter.toString(gpa);
    defer gpa.free(text);
    try t.expectEqualStrings("Counter(115)", text);
    try t.expectEqual(try counter.hash(), try twin.hash());

    try t.expectEqual(@as(i64, 100), try counter.trySub(15, null));
    try t.expectError(error.BadCode, counter.trySub(1000, null));

    const copy = counter.clone();
    defer copy.deinit();
    try t.expectEqual(@as(i64, 100), try copy.get());

    const holder = try tour.makeHolder(gpa, 1);
    defer holder.deinit(gpa);
    try t.expectEqual(@as(i64, 1), try holder.counter.get());
    try t.expectEqual(@as(i64, 3), try holder.others[1].get());
}

const Recording = struct {
    names: u32 = 0,
    last: [32]u8 = undefined,
    last_len: usize = 0,
    finished: bool = false,

    pub fn onEvent(self: *Recording, allocator: std.mem.Allocator, name: []const u8, value: i32) !i32 {
        _ = allocator;
        self.names += 1;
        @memcpy(self.last[0..name.len], name);
        self.last_len = name.len;
        return value * 10;
    }

    pub fn onDone(self: *Recording, allocator: std.mem.Allocator) !void {
        _ = allocator;
        self.finished = true;
    }

    pub fn fallible(self: *Recording, allocator: std.mem.Allocator, n: i32, err: *?tour.FlatError) ![]const u8 {
        _ = self;
        if (n < 0) {
            err.* = .{ .denied = .{ .v0 = try allocator.dupe(u8, "negative") } };
            return error.Denied;
        }
        if (n == 0) return error.NotFound; // a variant without fields, by name
        if (n == 1) return error.Unexpected;
        return std.fmt.allocPrint(allocator, "ok{d}", .{n});
    }
};

test "callback interfaces" {
    var rec: Recording = .{};
    const listener = tour.Listener.from(&rec);
    try t.expectEqual(@as(i32, 60), try tour.notify(listener, 4));
    try t.expect(rec.finished);
    try t.expectEqual(@as(u32, 4), rec.names);
    try t.expectEqualStrings("event-3", rec.last[0..rec.last_len]);

    const ok = try tour.tryListener(gpa, listener, 5, null);
    defer gpa.free(ok);
    try t.expectEqualStrings("ok5", ok);
    var err: tour.FlatError = undefined;
    try t.expectError(error.Denied, tour.tryListener(gpa, listener, -1, &err));
    defer err.deinit(gpa);
    try t.expectEqualStrings("negative", err.denied.v0);
    try t.expectError(error.NotFound, tour.tryListener(gpa, listener, 0, null));
    try t.expectError(error.RustPanic, tour.tryListener(gpa, listener, 1, null));
    try t.expect(std.mem.indexOf(u8, uniffi.lastErrorMessage(), "Unexpected") != null);
}

var released: u32 = 0;

const ZigGreeter = struct {
    who: []const u8,

    pub fn greet(self: *ZigGreeter, allocator: std.mem.Allocator, name: []const u8) ![]const u8 {
        return std.fmt.allocPrint(allocator, "{s} says hi to {s}", .{ self.who, name });
    }

    pub fn deinit(self: *ZigGreeter) void {
        _ = self;
        released += 1;
    }
};

test "traits implemented in Rust and in Zig" {
    const rust = try tour.rustGreeter();
    defer rust.deinit();
    const a = try rust.greet(gpa, "x");
    defer gpa.free(a);
    try t.expectEqualStrings("rust says hi to x", a);
    const b = try tour.useGreeter(gpa, rust, "y");
    defer gpa.free(b);
    try t.expectEqualStrings("rust says hi to y", b);

    var impl: ZigGreeter = .{ .who = "zig" };
    const z = tour.Greeter.fromImpl(&impl);
    const c = try z.greet(gpa, "w");
    defer gpa.free(c);
    try t.expectEqualStrings("zig says hi to w", c);
    const d = try tour.useGreeter(gpa, z, "v");
    defer gpa.free(d);
    try t.expectEqualStrings("zig says hi to v", d);
    const z2 = z.clone();
    z.deinit();
    try t.expectEqual(@as(u32, 0), released);
    z2.deinit();
    try t.expectEqual(@as(u32, 1), released);
}

const Source = struct {
    pub fn fetch(self: *Source, allocator: std.mem.Allocator, key: []const u8) ![]const u8 {
        _ = self;
        return std.fmt.allocPrint(allocator, "fetched {s}", .{key});
    }
};

test "async" {
    try t.expectEqual(@as(i32, 5), try tour.slowAdd(2, 3));
    const fine = try tour.slowFail(gpa, false, null);
    defer gpa.free(fine);
    try t.expectEqualStrings("fine", fine);
    try t.expectError(error.Plain, tour.slowFail(gpa, true, null));
    try tour.slowUnit();
    const worker = try tour.Worker.init("w");
    defer worker.deinit();
    const did = try worker.work(gpa, 3);
    defer gpa.free(did);
    try t.expectEqualStrings("w did 3", did);

    var source: Source = .{};
    const fetched = try tour.fetchVia(gpa, tour.AsyncSource.from(&source), "k");
    defer gpa.free(fetched);
    try t.expectEqualStrings("fetched k", fetched);
}

test "custom types" {
    try t.expectEqual(@as(u64, 42), try tour.nextHandle(41));
    const host = try tour.hostOf(gpa, "https://example.com/a/b");
    defer gpa.free(host);
    try t.expectEqualStrings("example.com", host);
}

test "uniffi 0.32: boxes, sets, lent bytes" {
    const two: tour.Expr = .{ .num = .{ .value = 2 } };
    const neg: tour.Expr = .{ .neg = .{ .inner = &two } };
    const five: tour.Expr = .{ .num = .{ .value = 5 } };
    try t.expectEqual(@as(i64, 3), try tour.eval(.{ .add = .{ .left = &neg, .right = &five } }));

    const sum = try tour.sumExpr(gpa, &.{ 1, 2, 3, 4 });
    defer sum.deinit(gpa);
    try t.expectEqual(@as(i64, 10), try tour.eval(sum));
    try t.expectEqual(@as(i64, 4), sum.add.right.num.value);

    const leaf: tour.TreeNode = .{ .label = "leaf", .children = &.{}, .first_sibling = null };
    const sibling = leaf;
    const tree: tour.TreeNode = .{ .label = "root", .children = &.{ leaf, leaf }, .first_sibling = &sibling };
    try t.expectEqual(@as(u32, 4), try tour.countNodes(tree));

    const words = try tour.uniqueWords(gpa, "a b a c b");
    defer {
        for (words) |w| gpa.free(w);
        gpa.free(words);
    }
    try t.expectEqual(@as(usize, 3), words.len);
    try t.expectEqual(@as(u32, 6), try tour.setSum(&.{ 1, 2, 3 }));

    const bytes = try gpa.alloc(u8, 100_000);
    defer gpa.free(bytes);
    @memset(bytes, 1);
    try t.expectEqual(@as(u32, 100_000), try tour.checksum(bytes));
    try t.expectEqual(@as(u32, 0), try tour.checksum(&.{}));
    const adler = try tour.Adler.init();
    defer adler.deinit();
    try adler.update(&.{ 1, 2 });
    try adler.update(&.{3});
    try t.expectEqual(@as(u32, (1 * 31 + 2) * 31 + 3), try adler.value());
}
