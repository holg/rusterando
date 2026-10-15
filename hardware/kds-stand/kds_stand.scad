// Counter stands for the kitchen display (STM32MP135F-DK, board MB1635).
//
// All board numbers come from ST's PCB file (MB1635.PcbDoc, board design
// package mb1635-bdp-v1-0): outline 127 x 97 mm, six M3 holes, the LCD
// (Rocktech RK043FN78H, 105.5 x 67.2, 4.5 mm high) and the plugs.
//
// Unit coordinates: looking at the screen, x right, y up, z towards you,
// origin = bottom-left corner of the board's front (screen) face. The
// screen side is the PCB's bottom layer, so x = 127 - x_pcb.
//
// Parts (PETG or ASA, not PLA — the pizza oven):
//   PART = "bezel"   front frame, screen window          (shared)
//   PART = "tub"     back shell, plug cut-outs           (shared)
//   PART = "stand_a" wedge, screen leaning back 20°
//   PART = "stand_c" heavy base + column, 35°
//   PART = "view_a" / "view_c"  assembled preview (not for printing)
//
//   openscad -D 'PART="tub"' -o tub.stl kds_stand.scad
//
// Hardware: 6x M3x25 (board + bezel, from the back), 4x M3x10 (shell to
// stand), 4 rubber feet; stand C also a ~1 kg steel plate in its base.

PART = "view_a";
$fn = 48;

// ---- board (from MB1635.PcbDoc) -------------------------------------------
BW = 127.0; BH = 97.0; BT = 1.6;
HOLES = [[123.4, 3.6], [123.4, 93.5], [90.2, 93.5], [32.2, 93.5],
         [3.5, 3.6], [3.5, 77.5]];
LCD = [7.6, 19.8, 113.1, 87.0];          // outline x0 y0 x1 y1
LCD_TOP = 4.5;                           // LCD face above board front
FRONT_PARTS = 8.4;                       // pin headers on the screen side
BACK_PARTS = 15.8;                       // stacked USB-A behind the board
WIN_C = [(LCD[0] + LCD[2]) / 2, (LCD[1] + LCD[3]) / 2];
WIN = [98, 58];                          // active area 95 x 54 + margin

// ---- shell ------------------------------------------------------------------
CL = 1.5;                                // board-to-wall clearance
WALL = 2.5;
FRONT_Z = FRONT_PARTS + 0.6;             // inside of bezel plate
BACK_Z = -(BT + BACK_PARTS + 1.6);       // inside of tub floor
X0 = -CL - WALL; X1 = BW + CL + WALL;
Y0 = -CL - WALL; Y1 = BH + CL + WALL;
R = 4;
BC = [BW / 2, BH / 2];                   // board centre
MOUNT = [for (sx = [-40, 40], sy = [-25, 25]) [BC[0] + sx, BC[1] + sy]];

module rbox(x0, y0, x1, y1, z0, z1, r = R) {
    hull() for (x = [x0 + r, x1 - r], y = [y0 + r, y1 - r])
        translate([x, y, z0]) cylinder(r = r, h = z1 - z0);
}

// Cut-outs through the walls, sized for the plugs, not just the sockets.
module plug_cuts() {
    // bottom edge: 2x stacked USB-A (printer), USB-C
    for (x = [29.0, 48.0]) translate([x - 9, Y0 - 1, BACK_Z + 0.5]) cube([18, 10, -BACK_Z - 0.5]);
    translate([11.0 - 7, Y0 - 1, -BT - 9]) cube([14, 10, 10]);
    // right edge: 2x Ethernet, ST-LINK USB-C
    for (y = [39.0, 61.7]) translate([BW - 5, y - 9.5, BACK_Z + 0.5]) cube([20, 19, -BACK_Z - 0.5]);
    translate([BW - 5, 14.3 - 7, -BT - 9]) cube([20, 14, 10]);
    // left edge: microSD (with finger room), micro-USB
    translate([X0 - 1, 65.1 - 9, -BT - 5]) cube([10, 18, 5.5]);
    translate([X0 - 1, 35.2 - 7, -0.5]) cube([10, 14, 5]);
    // top edge: camera flex
    translate([95, BH - 3, -BT - 4]) cube([24, 10, 4.2]);
}

module tub() {
    difference() {
        union() {
            difference() {
                rbox(X0, Y0, X1, Y1, BACK_Z - WALL, FRONT_Z);
                rbox(-CL, -CL, BW + CL, BH + CL, BACK_Z, FRONT_Z + 1, R - WALL);
            }
            // board bosses (board rests on them at z = -BT)
            for (h = HOLES) translate([h[0], h[1], BACK_Z - 0.1]) cylinder(d = 7, h = -BT - BACK_Z + 0.1);
            // stand bosses
            for (m = MOUNT) translate([m[0], m[1], BACK_Z - 0.1]) cylinder(d = 8, h = 4);
        }
        plug_cuts();
        // M3 through the bosses, head sunk into the back
        for (h = HOLES) translate([h[0], h[1], BACK_Z - WALL - 1]) {
            cylinder(d = 3.4, h = 40);
            cylinder(d = 6.5, h = 3.5);
        }
        for (m = MOUNT) translate([m[0], m[1], BACK_Z - WALL - 1]) cylinder(d = 2.6, h = 6.5);
        // a few vent slots up top at the back (the board makes ~1-2 W)
        for (i = [0:5]) translate([30 + i * 12, BH - 18, BACK_Z - WALL - 1]) rbox(0, 0, 4, 14, 0, WALL + 2, 1.9);
    }
}

module bezel() {
    t = 2.5;
    difference() {
        union() {
            rbox(X0, Y0, X1, Y1, FRONT_Z, FRONT_Z + t);
            // locating skirt inside the tub
            difference() {
                rbox(-CL + 0.2, -CL + 0.2, BW + CL - 0.2, BH + CL - 0.2, FRONT_Z - 1.5, FRONT_Z + 0.1, R - WALL);
                rbox(-CL + 1.4, -CL + 1.4, BW + CL - 1.4, BH + CL - 1.4, FRONT_Z - 2, FRONT_Z + 1, R - WALL);
            }
            // ring down onto the LCD (put 0.5 mm foam tape on it)
            translate([0, 0, LCD_TOP + 0.5]) linear_extrude(FRONT_Z - LCD_TOP - 0.4)
                offset(r = -1) polygon([[LCD[0], LCD[1]], [LCD[2], LCD[1]], [LCD[2], LCD[3]], [LCD[0], LCD[3]]]);
            // posts clamp the board from the front
            for (h = HOLES) translate([h[0], h[1], 0]) cylinder(d = 5.5, h = FRONT_Z + 0.1);
        }
        // window, chamfered towards the front so a finger reaches the edges
        translate([WIN_C[0], WIN_C[1], 0]) hull() {
            translate([0, 0, LCD_TOP]) cube([WIN[0], WIN[1], 1], center = true);
            translate([0, 0, FRONT_Z + 0.6]) cube([WIN[0], WIN[1], 0.01], center = true);
            translate([0, 0, FRONT_Z + t + 0.01]) cube([WIN[0] + 4, WIN[1] + 4, 0.01], center = true);
        }
        for (h = HOLES) translate([h[0], h[1], -1]) cylinder(d = 2.6, h = FRONT_Z);
    }
}

// ---- placing the display unit on a stand ---------------------------------
// World: X right, Y away from you, Z up. The unit pivots on the bottom
// edge of its shell, which sits LIFT above the counter, leaning back TILT.
module placed(tilt, lift) {
    translate([0, 0, lift]) rotate([-tilt, 0, 0]) rotate([90, 0, 0])
        translate([-BW / 2, -Y0, 0]) children();
}

// Plate behind the tub that the stand grows from (unit coordinates).
module mount_plate(xa = X0, xb = X1, th = 5) {
    translate([0, 0, BACK_Z - WALL - th])
        intersection() {
            rbox(X0, Y0 + 10, X1, Y1 - 6, 0, th);
            translate([xa, Y0, -1]) cube([xb - xa, Y1 - Y0, th + 2]);
        }
}

module mount_holes(tilt, lift) {
    placed(tilt, lift) for (m = MOUNT) translate([m[0], m[1], BACK_Z - WALL - 30]) {
        cylinder(d = 3.4, h = 40);
        cylinder(d = 6.5, h = 30 - 5 + 2.5);      // screw head pocket
    }
}

// A: wedge. Two side ribs + floor frame; the space under the screen stays
// open for the plugs on the bottom edge, cables leave at the back.
module stand_a(tilt = 20, lift = 60) {
    depth_back = 95; depth_front = -30;
    difference() {
        union() {
            placed(tilt, lift) mount_plate();
            for (s = [[X0, X0 + 7], [X1 - 7, X1]]) hull() {
                placed(tilt, lift) mount_plate(s[0], s[1]);
                translate([s[0] - BW / 2 - Y0 * 0 + CL * 0 - X0 + X0, depth_front, 0])
                    cube([s[1] - s[0], depth_back - depth_front, 5]);
            }
            // floor frame: side rails are in the ribs, add back + front bars
            translate([X0 - BW / 2 + CL * 0, depth_back - 14, 0]) cube([X1 - X0, 14, 5]);
            translate([X0 - BW / 2, depth_front, 0]) cube([X1 - X0, 10, 5]);
        }
        mount_holes(tilt, lift);
        // cable exit through the back bar
        translate([-45, depth_back - 20, -1]) rbox(0, 0, 70, 30, 0, 12, 3);
    }
}

// C: heavy base + column. The column sits right of centre — every plug on
// the bottom edge is on the left half (USB-C power, 2x USB-A).
module stand_c(tilt = 35, lift = 75) {
    bx = 170; by = 150; bz = 14; boff = 25;
    difference() {
        union() {
            translate([-bx / 2, -by / 2 + boff, 0]) rbox(0, 0, bx, by, 0, bz, 8);
            placed(tilt, lift) mount_plate();
            hull() {
                placed(tilt, lift) mount_plate(BW / 2 + 5, BW / 2 + 50, 5);
                translate([5, 30, 0]) cube([45, 40, bz]);
            }
        }
        mount_holes(tilt, lift);
        // pocket underneath for a steel plate / weights (glue in, then feet)
        translate([-bx / 2 + 8, -by / 2 + boff + 8, -1]) rbox(0, 0, bx - 16, by - 16, 0, bz - 3.5 + 1, 4);
    }
}

module unit() { color("#9aa0a6") tub(); color("#c9ccd0") bezel();
    color("#111") translate([LCD[0], LCD[1], 0]) cube([LCD[2] - LCD[0], LCD[3] - LCD[1], LCD_TOP]);
    color("#2a6df5") translate([WIN_C[0] - 47.5, WIN_C[1] - 27, LCD_TOP]) cube([95, 54, 0.2]);
    color("#2f7d3a") translate([0, 0, -BT]) cube([BW, BH, BT]); }

if (PART == "bezel") rotate([180, 0, 0]) bezel();       // print face down
else if (PART == "tub") tub();
else if (PART == "stand_a") stand_a();
else if (PART == "stand_c") stand_c();
else if (PART == "view_a") { color("#7d848c") stand_a(); placed(20, 60) unit(); }
else if (PART == "view_c") { color("#7d848c") stand_c(); placed(35, 75) unit(); }
