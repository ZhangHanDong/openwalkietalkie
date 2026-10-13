# openwalkietalkie.org

The official OpenWalkieTalkie website, sharing OpenMicroKbd's pixel-art day
and night theme. It includes an interactive deck, firmware and hardware
renders, Mac downloads, and release posts with an Atom feed.

## Preview

No build step is required. From this directory:

```bash
python3 -m http.server 8787 -d public
```

For Cloudflare routing, security headers, and the custom 404 page:

```bash
npx wrangler@4.120.1 dev
```

## Deploy

The site uses Cloudflare Workers static assets, matching openmicrokbd.org.
The `wrangler.jsonc` routes provision HTTPS for `openwalkietalkie.org` and
`www.openwalkietalkie.org` on the CoNoL Cloudflare account.

```bash
cd site
npx wrangler@4.120.1 login --browser=false  # only when the login has expired
npx wrangler@4.120.1 deploy --dry-run
npx wrangler@4.120.1 deploy
```

Open the printed login URL in the browser profile that holds the Cloudflare
account. Both hostnames serve the site; canonical URLs use the apex domain.
The workers.dev URL provides a deployment preview.

## Content and releases

- `public/index.html`: product information, interactive deck, gallery, downloads.
- `public/js/main.js`: theme, demo, PTT simulation, and galleries. The browser
  demo uses scripted text; it does not record audio or connect to real agents.
- `public/blog/`: posts and Atom feed. Update `sitemap.xml` when adding a post.
- `public/assets/screens/`: firmware renders from `tools/preview-all.py`.
- `public/_headers`: security headers and caching. Mutable CSS and JavaScript
  revalidate on each visit, so a deployment does not leave stale behavior.

When shipping a new version, update the download section's versioned GitHub
asset URLs and release copy, add a post/feed entry, then deploy after the
GitHub release is public. Versioned URLs keep old website copies working
when a newer release becomes latest.

Product photos and video can replace the existing hardware renders when
available. The film player is dormant until film markup and a YouTube ID
are added. No analytics property is configured.

## Licenses

Site code uses the repository's MIT license. The bundled Press Start 2P and
VT323 fonts keep their SIL Open Font License notices in `public/fonts/`.
Hardware renders come from the repository's CERN-OHL-P-2.0 hardware designs;
the corresponding notice is included as `public/assets/HARDWARE-LICENSE.txt`.
