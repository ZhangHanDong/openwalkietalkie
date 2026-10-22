#!/bin/sh
# Product photos and design renders for the website, from the team's originals
# in ../ref/photos. Photos are auto-oriented, cropped to the window the page
# shows (3:4 for the hardware stack, 4:5 for the hero), resized for a 2x
# display, stripped of all metadata (the originals carry GPS coordinates) and
# written as WebP. The two colourway turnarounds are converted as they are.
#
#   sh tools/photos.sh      # → public/assets/photos/*.webp, public/assets/renders/*.webp
#
# Crops are hand-picked per photo so the device sits where the card expects
# it and nobody's face is in frame. Needs ImageMagick 7 (brew install imagemagick).
set -eu
cd "$(dirname "$0")/.."
SRC=../ref/photos

shot() { # name source crop(WxH+X+Y) size
  magick "$SRC/$2" -auto-orient -crop "$3" +repage -resize "$4" -strip -quality 82 -define webp:method=6 "public/assets/photos/$1.webp"
  printf '%s  ' "public/assets/photos/$1.webp"; magick identify -format '%wx%h\n' "public/assets/photos/$1.webp"
}
render() { # name source
  magick "$SRC/$2" -strip -quality 85 -define webp:method=6 "public/assets/renders/$1.webp"
  printf '%s  ' "public/assets/renders/$1.webp"; magick identify -format '%wx%h\n' "public/assets/renders/$1.webp"
}

# hero: standing on the bench, Usage face (4:5)
shot hero-usage  DSCF7285.JPG 1045x1306+0+217 1045x1306
# hardware stack (3:4), in the order the page shows them
shot on-the-desk DSCF7277.JPG 1134x1512+0+124 750x1000
shot in-hand     DSCF7280.JPG 1184x1579+0+110 750x1000
shot session     DSCF7281.JPG  940x1253+0+139 750x1000
shot hold        DSCF7306.JPG  840x1120+0+230 750x1000
shot talk        DSCF7314.JPG  593x791+240+0  750x1000
# colourway turnarounds (front/left, rear/right, top, bottom)
render black-green  黑绿.png
render white-orange 白橙配色.png
