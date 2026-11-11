//! The Outlook appointment happening right now: subject and the other people.
//! Used when a meeting starts, so the notes can say Bob instead of "Them".
//! Outlook has to already be open. A closed Outlook is left closed.
use crate::smart::{self, Appointment};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const SCRIPT: &str = r#"
$ErrorActionPreference = 'SilentlyContinue'
if (-not (Get-Process OUTLOOK -ErrorAction SilentlyContinue)) { exit 0 }
$ol = New-Object -ComObject Outlook.Application
$ns = $ol.GetNamespace('MAPI')
$cal = $ns.GetDefaultFolder(9)
$items = $cal.Items
$items.IncludeRecurrences = $true
$items.Sort('[Start]')
$now = Get-Date
$from = $now.AddHours(-8)
$to = $now.AddHours(8)
$count = 0
foreach ($item in $items) {
  $count++
  if ($count -gt 400) { break }
  try {
    if ($item.Start -gt $to) { break }
    if ($item.End -lt $from) { continue }
    if ($item.Start -le $now -and $item.End -ge $now -and -not $item.AllDayEvent) {
      $names = @()
      foreach ($r in @($item.Recipients)) { if ($r.Name) { $names += [string]$r.Name } }
      [pscustomobject]@{ subject = [string]$item.Subject; people = $names } | ConvertTo-Json -Compress
      exit 0
    }
  } catch {}
}
exit 0
"#;

/// The appointment that contains the current time, or nothing if Outlook is closed or has no match.
pub fn current(user_name: &str, email: &str) -> Option<Appointment> {
    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(4) {
            let _ = child.kill();
            return None;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut out = String::new();
                child.stdout.as_mut()?.read_to_string(&mut out).ok()?;
                let parsed = smart::parse_appointment(&out)?;
                let people = smart::other_people(&parsed.people, user_name, email);
                if parsed.subject.is_empty() && people.is_empty() {
                    return None;
                }
                return Some(Appointment { subject: parsed.subject, people });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => return None,
        }
    }
}
