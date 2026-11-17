# Wisp and Windows antivirus

Wisp's Windows build is not code-signed yet. Windows treats new unsigned programs
with suspicion, so users may see a SmartScreen prompt or, rarely, a Microsoft
Defender warning. This page says what the project does about it and what only the
maintainer can do.

## What happened with 0.2.1

After 0.2.0 updated itself to 0.2.1, Defender's real-time protection quarantined
the new `wisp.exe` as `Trojan:Win32/Bearfoos.A!ml`, a verdict of its local
machine-learning model, and Windows refused to start it ("the file contains a virus
or potentially unwanted software"). On the maintainer's PC, Protection history
showed a severe Trojan ("executes commands from an attacker", Microsoft's
standard wording for this category) a few minutes after 0.2.1 was published,
first against the running process, and Defender removed the whole installation
(program, shortcuts, uninstall entry). This was reproduced on GitHub's Windows
runners:

- A scan of the same file, with current definitions and cloud lookups, finds
  nothing, and installing 0.2.1 directly and starting it is never flagged. The
  verdict comes when Wisp is started after it was installed by an update, and only
  in about half the runs.
- Whether the installer was started through hidden PowerShell (as in 0.2.0 and
  0.2.1) or directly made no measurable difference (5 of 9 runs flagged against 4
  of 6). Giving `wisp.exe` its version information, icon and manifest did: 0
  detections in 12 update runs, against 16 in 29 for builds without them (the
  published 0.2.1, a fresh build of the same code, and this change without the
  resources).
- The published files were also checked for anything unwanted: their SHA-256
  digests match what the release workflow built, and rebuilding the 0.2.1 tag
  gives a `wisp.exe` that differs from the published one only in its link
  timestamp and build ID (24 bytes).

## What the build does

- **`wisp.exe` says what it is.** It carries a version resource (name, version,
  publisher, copyright), the Wisp icon and an application manifest (runs as the
  user, never asks for administrator rights, supports Windows 10 and 11, sharp on
  high-resolution screens). See [`build.rs`](../build.rs).
- **Updates start the installer directly**, after checking its signature against
  the key built into the app, instead of through a hidden PowerShell window. It is
  saved as `Wisp-<version>-Setup.exe` in the user's temp folder and shows its own
  progress window; a clear error replaces the old crash if it cannot start.
- **Every Windows release is checked before it is published.** The release
  workflow runs [`.github/defender-check.ps1`](../.github/defender-check.ps1) on
  five fresh Windows runners at once. Each turns real-time protection on, proves
  it works on the harmless EICAR test file, and then rehearses an automatic
  update twice: a copy of Wisp built from the same commit (with a throwaway update
  key) downloads the new installer from a local server, checks its signature,
  installs it on quit, and the installed Wisp is started again, twice. Then
  `wisp.exe` and the installer are scanned with fresh definitions. Any detection,
  or a Wisp that cannot start, stops the release. The SHA-256 of each file is in
  the run's summary, so a file on someone's computer can be compared with what
  was built.

What no build machine can see: SmartScreen's reputation prompt ("Windows protected
your PC"), and verdicts that depend on how many people have run a file. Machine
learning verdicts are also not certain: ten clean rehearsals on five machines make
a flagged build unlikely to slip through, not impossible. A release that fails the
check can be retried, but a repeated failure means Windows users would see it too.

## Windows automatic updates: off until checked

Release manifests (`latest.json`) include Windows only when the repository
variable `WISP_WINDOWS_UPDATES` is `on` (Settings > Secrets and variables >
Actions > Variables). Without it, installed Windows copies find no update and
carry on as they are; macOS is unaffected. The 0.2.1 and 0.3.0 manifests have no
Windows entry.

Before setting it to `on`, check on a real Windows 10 or 11 PC with Microsoft
Defender at its defaults (real-time protection and cloud protection on):

1. Install the first release made with this check (from its release page) and
   start it. In **Windows Security > Protection history** nothing about Wisp
   appears.
2. In File Explorer, right-click `%LOCALAPPDATA%\Wisp\wisp.exe` > Properties >
   Details: File description "Wisp", product name "Wisp", copyright "Siimas",
   and the version of that release. (0.3.0 and earlier show these empty.)
3. Check the release's **Microsoft Defender check** jobs passed (they rehearse the
   update itself on five Windows machines; a PC cannot, since with the variable
   off there is no update to take).
4. Set `WISP_WINDOWS_UPDATES` to `on`. It takes effect from the next release (the
   variable is read when a release is published), and that release's update is
   the first real one: keep one PC on the previous release, let it update (a
   small installer window appears and Wisp starts again), and start Wisp a few
   times over the next hour. If Protection history shows anything, set the
   variable back to off and delete the Windows entry from that release's
   `latest.json`, as was done for 0.2.1 and 0.3.0.

If anything is flagged at any step, leave the variable off, note the detection
name, and see below.

## If Defender flags Wisp

Do not turn protection off or add an exclusion. Instead:

1. **Read the detection name.** Open **Windows Security > Virus & threat
   protection > Protection history**, click the Wisp entry, and note
   **Detected:** (for 0.2.1 it was `Trojan:Win32/Bearfoos.A!ml`), the
   **affected items** path, and the date. A name ending in `!ml` is a verdict of
   Defender's machine-learning models, the usual cause of false positives on new
   unsigned programs. Leave the file in quarantine for now.
2. **Check the file is the one the project built.** In PowerShell:
   `Get-FileHash "$env:LOCALAPPDATA\Wisp\wisp.exe"` (or the Setup.exe you
   downloaded) and compare with the SHA-256 in the release workflow's summary
   for that version, or with the digest GitHub shows next to each release asset.
3. **Report it to Microsoft as a false positive** (maintainer, or any user):
   go to <https://www.microsoft.com/en-us/wdsi/filesubmission>, sign in with a
   Microsoft account, choose **Home customer** (or **Software developer**, which
   gets a faster, tracked review and needs the developer's Microsoft account),
   choose **Microsoft Defender Antivirus** as the product, upload the flagged file
   (the installer from the release page is best, since it contains `wisp.exe`),
   select **Incorrectly detected as malware/malicious**, give the detection name
   from step 1 and the SHA-256 of the flagged `wisp.exe`, and say it is the
   open-source Wisp browser built by GitHub Actions from
   <https://github.com/Siimas/wisp-browser>. For 0.2.1 that is the installer
   `Wisp-0.2.1-Windows-x64-Setup.exe` (SHA-256 `3edd813e…e4788`) containing
   `wisp.exe` with SHA-256
   `71240ae53b5e89338b49647762bb153e1f7a571cc8b54057e5050afeec285e32`, flagged
   as `Trojan:Win32/Bearfoos.A!ml` after an automatic update. Microsoft usually
   answers within a few days; when they clear it, they publish new definitions,
   after which `MpCmdRun.exe -SignatureUpdate` (or Windows Update) removes the
   warning, and the file can be restored from Protection history.

The project never uploads files to scanning services automatically.

## Code signing

An Authenticode signature ties `wisp.exe` and the installer to a verified
publisher. Defender and SmartScreen build reputation for a *publisher*, not just
for each file, so once a certificate has a clean history, new versions are
trusted from the first download instead of starting from zero every release.
It does not make warnings impossible: a new certificate starts with no reputation,
so the first signed releases can still see SmartScreen prompts until enough
people have installed them.

Realistic options, cheapest first:

- **Azure Trusted Signing** (Microsoft): a low monthly subscription, signs from
  GitHub Actions without handling a key file; needs identity validation, and
  eligibility has been limited by country and for individuals, so check it first.
- **SignPath Foundation**: free code signing for open-source projects that meet
  its conditions (builds on a public CI from public source, which Wisp does); the
  certificate is issued to the foundation, with the project named.
- **An OV code-signing certificate** from a certificate authority (yearly; the
  key must live on a hardware token or a cloud HSM, so CI signing needs the CA's
  cloud signing service).
- **An EV certificate**: more expensive; since 2024 it no longer skips the
  SmartScreen reputation period, so it is not worth the premium for Wisp.

Signing would be added to the Windows job of the release workflow: `wisp.exe`
after the build and before packaging, then the installer, both before the
Defender check.
