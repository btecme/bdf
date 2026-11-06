BDF portable package
====================

Run it
------
Extract this ZIP anywhere you can write to (for example your Documents or
%LOCALAPPDATA%\Programs\BDF) and open bdf.exe. Nothing is installed, no administrator
rights are needed, and nothing is written outside this folder unless you ask for it.

Your data
---------
While portable.txt sits next to bdf.exe, BDF keeps your settings, log files and
crash-recovery files in the BDFData folder beside it. Updates never touch BDFData.
Copy the whole folder to move BDF with its settings.

Make BDF your default PDF app (optional)
----------------------------------------
Help > Windows integration > "Make BDF my default PDF app" registers this copy with
Windows for your account only and opens Windows' Default apps page, where you confirm
the choice. "Remove Windows integration" undoes it. If you move this folder, use
"Refresh Windows integration" there. A copy that was only extracted never touches the
registry.

Updates
-------
When a newer BDF release is available a green "Update" button appears at the top right.
Choosing it downloads the release, checks its signature and contents, asks you to save any
unsaved work, closes BDF, replaces the program files and opens BDF again. The previous
version is kept in the BDFPrevious folder, and BDF can go back to it. Updates can be turned
off in Preferences. You can also check by hand with Help > Check for updates.

Folders BDF manages inside this one: BDFData (yours), BDFUpdate (update working files),
BDFPrevious (the version before the last update). Anything else you put here is left alone.

Command line
------------
bdf-cli.exe is the command-line tool (bdf-cli update status, bdf-cli windows status, ...).
bdf-worker.exe and bdf-updater.exe are helpers started by BDF; do not run them by hand.

Licences are in LICENSE-MIT, LICENSE-APACHE, NOTICE and the other licence files here.
https://github.com/btecme/bdf
