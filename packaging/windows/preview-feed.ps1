<#
.SYNOPSIS
  Serves a folder over http://127.0.0.1:<port>/ so a development preview of BDF can be shown an update (BUILD-018).
.DESCRIPTION
  A development build (made with BDF_UPDATE_CHANNEL=dev) reads its release feed from BDF_UPDATE_FEED and trusts only
  its test signing key. Serve a folder holding release.json (a bdf-update.json made by `cargo xtask update-sign`) and the
  package it names, set BDF_UPDATE_FEED=http://127.0.0.1:<port>/release.json in the shell that starts bdf.exe, and the
  green Update button appears when the release is newer. Nothing leaves this machine. Stop with Ctrl+C.
#>
param(
    [Parameter(Mandatory)][string]$Folder,
    [int]$Port = 8089
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path $Folder).Path
$listener = New-Object System.Net.HttpListener
# Loopback only: this is a local test server, not a service.
$listener.Prefixes.Add("http://127.0.0.1:$Port/")
$listener.Start()
Write-Output "Serving $root on http://127.0.0.1:$Port/  (feed: http://127.0.0.1:$Port/release.json). Ctrl+C to stop."
try {
    while ($listener.IsListening) {
        $ctx = $listener.GetContext()
        try {
            $rel = [Uri]::UnescapeDataString($ctx.Request.Url.AbsolutePath.TrimStart('/'))
            $path = [IO.Path]::GetFullPath((Join-Path $root $rel))
            if ($path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $path -PathType Leaf)) {
                $bytes = [IO.File]::ReadAllBytes($path)
                $ctx.Response.ContentLength64 = $bytes.Length
                $ctx.Response.OutputStream.Write($bytes, 0, $bytes.Length)
            } else {
                $ctx.Response.StatusCode = 404
            }
            $ctx.Response.Close()
        } catch {
            # A client that gave up part-way (a download that failed on a full disk, say) must not end the server.
            try { $ctx.Response.Abort() } catch { }
        }
    }
} finally { $listener.Stop() }
