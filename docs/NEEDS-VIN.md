# Needs Vin

Owner-only steps, in the order they're needed. Everything else is handled without you.

## Now (before the Store name is reserved)
1. **Trademark searches for "Editelier"**, classes **9** and **42**. Automated search was blocked on these three; USPTO, Store, GitHub and the web came back clear.
   - IPOS: https://digitalhub.ipos.gov.sg
   - WIPO Global Brand Database: https://branddb.wipo.int
   - EUIPO TMview: https://www.tmdn.org/tmview
   - Report back: "clear" or the conflicting mark.
2. **Partner Center individual developer account** (free, ID + selfie verification): https://storedeveloper.microsoft.com/
3. **Reserve the app name "Editelier"** in Partner Center (Apps and games → New product → MSIX or PWA app). Then send me the Product identity values (Package/Identity/Name, Publisher, PublisherDisplayName) for the manifest.

4a. **Trust the dev test certificate** (once, unblocks the install tests). Open PowerShell as administrator and run:
   `Import-Certificate -FilePath "$env:LOCALAPPDATA\Editelier-dev\editelier-dev.cer" -CertStoreLocation Cert:\LocalMachine\TrustedPeople`
   It only trusts packages signed by our local dev key on this laptop.

## Optional
4. **Domain** (costs money, your call): editelier.app and editelier.dev were free on 2026-11-25; editelier.com is taken. Without one, the landing page uses a `*.pages.dev` address.
5. **Test footage**: a few phone clips with no people in them: 4K HEVC (HDR on), 1080p normal, a short screen recording. Drop them in `test-media/owner/` in the repo folder (gitignored, never published).
6. **HEVC Video Extensions** on the test laptop if not already installed ($0.99, Microsoft Store), needed to test HEVC import/export.

## Later
7. **Cloudflare auth** for the landing page (after M1): run `npx wrangler login` once in a terminal, or tell me it's already done.
8. **Store listing submission** (after M1): approve the listing text and screenshots I prepare, complete the IARC age-rating questionnaire in Partner Center, and press Submit.
9. ~~OK to make the repo public~~ Done 2026-11-25 (public at creation, gate passed).
10. **Optional: commission a real Editelier logo.** The current mark (`brand/`) is a placeholder made in-house (D22).
