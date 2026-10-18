#!/bin/sh
# Render the app icon and the menu bar icon from the source artwork in assets/.
#
#   scripts/make-icon.sh    -> assets/AppIcon.icns, docs/icon.png and
#                              assets/StatusIcon.tiff, assets/StatusIconIdle.tiff
#
# Layout follows Apple's macOS app icon template: a 1024 px canvas, an 824 px
# rounded square (corner radius 185) with a soft drop shadow, and the artwork
# centred on it. Every .icns slot is downsampled from that master in linear
# light with a Lanczos filter and written as a stripped, maximally compressed
# PNG. The menu bar icons are the pill alone, in colour while keeping awake and
# grey and faded while idle, each a TIFF holding the 1x and 2x images. Needs
# ImageMagick 7 (`brew install imagemagick`); iconutil ships with macOS. The
# outputs are committed, so building the bundle needs neither.
set -eu
cd "$(dirname "$0")/.."

SRC=assets/bluemeth-open-seam-enamel.png
ICNS=assets/AppIcon.icns
PREVIEW=docs/icon.png
STATUS_ACTIVE=assets/StatusIcon.tiff
STATUS_IDLE=assets/StatusIconIdle.tiff

command -v magick >/dev/null 2>&1 || {
    echo "make-icon.sh: ImageMagick 7 (magick) is required: brew install imagemagick" >&2
    exit 1
}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

CANVAS=1024
PLATE=824       # rounded square ("plate") size
PLATE_POS=100   # (CANVAS - PLATE) / 2
RADIUS=185
PILL=640        # longest side of the pill's solid bounding box on the canvas
PAD=60          # room for the shadow blur around the plate
SHADOW_DY=12
STATUS=18       # menu bar icon size in points
STATUS_PILL=17  # longest side of the pill's solid bounding box in the menu bar, in points
IDLE_ALPHA=0.5  # opacity of the idle menu bar icon

# Deterministic, small PNGs: no metadata, no timestamps, zlib level 9.
PNG="-depth 8 -strip -define png:compression-level=9 -define png:compression-filter=5 -define png:exclude-chunk=date,time"

# --- plate: diagonal gradient with a faint highlight, rounded corners ------
magick -size ${PLATE}x${PLATE} -define gradient:angle=155 gradient:'#2b55ad-#0a1130' \
    \( -size ${PLATE}x${PLATE} radial-gradient:'rgba(255,255,255,0.16)-rgba(255,255,255,0)' \) \
    -geometry +0-230 -composite \
    \( -size ${PLATE}x${PLATE} xc:none -fill white \
       -draw "roundrectangle 0,0 $((PLATE - 1)),$((PLATE - 1)) ${RADIUS},${RADIUS}" \) \
    -alpha off -compose CopyOpacity -composite \
    "$WORK/plate.png"

# --- drop shadow: the plate's silhouette, black at 35 %, blurred ------------
magick "$WORK/plate.png" -bordercolor none -border ${PAD} \
    -fill black -colorize 100 -channel A -evaluate multiply 0.35 +channel \
    -blur 0x14 "$WORK/shadow.png"

magick -size ${CANVAS}x${CANVAS} xc:none \
    "$WORK/shadow.png" -geometry +$((PLATE_POS - PAD))+$((PLATE_POS - PAD + SHADOW_DY)) -composite \
    "$WORK/plate.png" -geometry +${PLATE_POS}+${PLATE_POS} -composite \
    "$WORK/base.png"

# --- artwork: scale so its solid bounding box is PILL px, centre that box ---
BBOX=$(magick "$SRC" -channel A -threshold 50% +channel -format '%@' info:)   # WxH+X+Y
BW=${BBOX%%x*}; REST=${BBOX#*x}
BH=${REST%%+*};  REST=${REST#*+}
BX=${REST%%+*};  BY=${REST#*+}
SW=$(magick "$SRC" -format '%w' info:)
SH=$(magick "$SRC" -format '%h' info:)

GEOM=$(awk -v pill="$PILL" -v sw="$SW" -v sh="$SH" -v bw="$BW" -v bh="$BH" \
           -v bx="$BX" -v by="$BY" -v c="$CANVAS" 'BEGIN {
    m = (bw > bh) ? bw : bh; s = pill / m
    printf "%dx%d! %+d%+d\n", sw * s + 0.5, sh * s + 0.5,
           c / 2 - (bx + bw / 2) * s + 0.5, c / 2 - (by + bh / 2) * s + 0.5
}')
SIZE=${GEOM% *}
OFFSET=${GEOM#* }

# shellcheck disable=SC2086  # $PNG is a list of options on purpose
magick "$WORK/base.png" \
    \( "$SRC" -colorspace RGB -filter Lanczos -resize "$SIZE" -colorspace sRGB \) \
    -geometry "$OFFSET" -composite $PNG "$WORK/icon_1024.png"

# --- iconset -> icns ---------------------------------------------------------
ICONSET="$WORK/AppIcon.iconset"
mkdir "$ICONSET"
render() { # render <px> <file>
    # shellcheck disable=SC2086
    magick "$WORK/icon_1024.png" -colorspace RGB -filter Lanczos -resize "$1x$1" \
        -colorspace sRGB $PNG "$ICONSET/$2"
}
render 16  icon_16x16.png
render 32  icon_32x32.png;   cp "$ICONSET/icon_32x32.png"   "$ICONSET/icon_16x16@2x.png"
render 64  icon_32x32@2x.png
render 128 icon_128x128.png
render 256 icon_256x256.png; cp "$ICONSET/icon_256x256.png" "$ICONSET/icon_128x128@2x.png"
render 512 icon_512x512.png; cp "$ICONSET/icon_512x512.png" "$ICONSET/icon_256x256@2x.png"
cp "$WORK/icon_1024.png" "$ICONSET/icon_512x512@2x.png"

iconutil -c icns "$ICONSET" -o "$ICNS"
cp "$ICONSET/icon_256x256.png" "$PREVIEW"

# --- menu bar icons: a square around the pill's solid bounding box ----------
SIDE=$(awk -v bw="$BW" -v bh="$BH" -v s="$STATUS" -v p="$STATUS_PILL" \
           'BEGIN { m = (bw > bh) ? bw : bh; printf "%d\n", m * s / p + 0.5 }')
SX=$((BX + BW / 2 - SIDE / 2))
SY=$((BY + BH / 2 - SIDE / 2))
magick "$SRC" -crop "${SIDE}x${SIDE}+${SX}+${SY}" +repage "$WORK/pill.png"
status() { # status <output> [extra magick options]
    out=$1; shift
    for scale in 1 2; do
        magick "$WORK/pill.png" -colorspace RGB -filter Lanczos -resize "$((STATUS * scale))x$((STATUS * scale))" \
            -colorspace sRGB "$@" -set units PixelsPerInch -set density $((72 * scale)) "$WORK/status@$scale.png"
    done
    # TIFF keeps both resolutions in one file; NSImage picks the one the screen needs
    magick "$WORK/status@1.png" "$WORK/status@2.png" -depth 8 -strip -compress lzw -define tiff:predictor=2 "$out"
}
status "$STATUS_ACTIVE"
status "$STATUS_IDLE" -colorspace Gray -colorspace sRGB -channel A -evaluate multiply "$IDLE_ALPHA" +channel

echo "source  $SRC: ${SW}x${SH}, solid bbox ${BBOX}"
echo "artwork placed at ${OFFSET} scaled to ${SIZE%!}"
echo "wrote   $ICNS ($(du -h "$ICNS" | cut -f1)), $PREVIEW ($(du -h "$PREVIEW" | cut -f1))"
echo "wrote   $STATUS_ACTIVE, $STATUS_IDLE (${STATUS} pt, pill ${STATUS_PILL} pt)"
