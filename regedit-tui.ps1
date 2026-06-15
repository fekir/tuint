using namespace Microsoft.Win32

class ScreenBuffer {
  [Parameter(Mandatory)][char[][]]$Cells
  [Parameter(Mandatory)][int]$Width
  [Parameter(Mandatory)][int]$Height

  ScreenBuffer([int]$width, [int]$height) {
    $this.Width  = $width;
    $this.Height = $height;
    $this.Cells  = [char[][]]::new($height);
    $blank = [string]::new(' ', $width).ToCharArray();
    for ($y = 0; $y -lt $height; $y++) {
      $this.Cells[$y] = $blank.Clone();
    }
  }

  ScreenBuffer([ScreenBuffer]$other) {
    $this.Width  = $other.Width;
    $this.Height = $other.Height;
    $this.Cells  = [char[][]]::new($this.Height);

    for ($y = 0; $y -lt $this.Height; $y++) {
      $this.Cells[$y] = [char[]]::new($this.Width);
      [Array]::Copy($other.Cells[$y], $this.Cells[$y], $this.Width);
    }
  }

  [void]SetLine([int]$y, [string]$text) {
    if ($y -lt 0 -or $y -ge $this.Height) { return }
    $line = $text.PadRight($this.Width)
    $line.CopyTo(0, $this.Cells[$y], 0, $this.Width)
  }
}

function Init-Buffers {
  param(
    [Parameter(Mandatory)][ref]$old,
    [Parameter(Mandatory)][ref]$new
  )
  $width  = [Math]::Max(1, $raw.WindowSize.Width);
  $height = [Math]::Max(1, $raw.WindowSize.Height);
  $old.Value = [ScreenBuffer]::new($width, $height);
  $new.Value = [ScreenBuffer]::new($width, $height);
  clear;
}

function Print-Diff {
  param(
    [Parameter(Mandatory)][ScreenBuffer]$Old,
    [Parameter(Mandatory)][ScreenBuffer]$New
  )

  for ($y = 0; $y -lt $New.Height; $y++) {
    $rowOld = $Old.Cells[$y];
    $rowNew = $New.Cells[$y];
    if([System.Linq.Enumerable]::SequenceEqual($rowOld, $rowNew)){ continue; }
    $width  = $New.Width;
    $x = 0;
    while ($x -lt $width) {

      # Skip unchanged cells
      if ($rowOld[$x] -ceq $rowNew[$x]) {
        $x++;
        continue;
      }
      # Found a changed region - find its end
      $start = $x;
      while ($x -lt $width -and $rowOld[$x] -cne $rowNew[$x]) {
        $x++;
      }

      $length = $x - $start;

      [Console]::SetCursorPosition($start, $y);
      [Console]::Out.Write($rowNew, $start, $length);
      [Array]::Copy($rowNew, $start, $rowOld, $start, $length);
    }
  }
}

function Show-ChooseDialog {
  param(
    [Parameter(Mandatory)][ScreenBuffer]$OldBuf,
    [Parameter(Mandatory)][ScreenBuffer]$NewBuf,
    [string]$Message = "Are you sure?",
    [string[]]$Buttons = @("Yes", "No", "Cancel")
  )

  # Build button preview line
  $btnLine = ""
  foreach ($b in $Buttons) {
    $btnLine += "  $b   "
  }
  $width = [Math]::Max($btnLine.Length, $Message.Length) + 4
  $height = 7;

  $left = [Math]::Floor(($NewBuf.Width  - $width) / 2);
  $top  = [Math]::Floor(($NewBuf.Height - $height) / 2);

  $sel = 0;

  $NewBuf.SetLine($top+0, "/" + ("~" * ($width - 2)) + "\");
  $NewBuf.SetLine($top+1, "|" + (" " * ($width - 2)) + "|");
  $pad1 = [Math]::Floor(($width - $Message.Length) / 2) - 1;
  $pad2 = [Math]::Ceiling(($width - $Message.Length) / 2) - 1;
  $NewBuf.SetLine($top+2, "|" + (" " * $pad1) + $Message + (" " * $pad2) + "|" );
  $NewBuf.SetLine($top+3, "|" + (" " * ($width - 2)) + "|");
  $NewBuf.SetLine($top+5, "|" + (" " * ($width - 2)) + "|");
  $NewBuf.SetLine($top+6, "\" + ("~" * ($width - 2)) + "/");

  while ($true) {
    $btnLine = "";
    for ($i = 0; $i -lt $Buttons.Count; $i++) {
        if ($i -eq $sel) {
            $btnLine += "[ $($Buttons[$i]) ] ";
        } else {
            $btnLine += "  $($Buttons[$i])   ";
        }
    }

    # Center buttons
    $pad = [Math]::Floor(($width - $btnLine.Length) / 2) - 1;
    $NewBuf.SetLine($top + 4, "|" + (" " * $pad) + $btnLine + (" " * $pad) + "|");

    Print-Diff -Old $OldBuf -New $NewBuf;

    $key = [Console]::ReadKey($true);
    switch -CaseSensitive ($key.Key) {
        'LeftArrow'  { $sel = ($sel - 1 + $Buttons.Count) % $Buttons.Count; }
        'RightArrow' { $sel = ($sel + 1                 ) % $Buttons.Count; }
        'Enter'      { return $Buttons[$sel]; }
        'Escape'     { return "Cancel"; }
        'Q'          { return "Cancel"; }
    }
  }
}

function Invoke-InlineEditor {
  param(
    [Parameter(Mandatory)][ScreenBuffer]$OldBuf,
    [Parameter(Mandatory)][ScreenBuffer]$NewBuf,
    [int]$Row = 0,
    [Parameter(Mandatory)][string]$Description,
    [string]$Prompt = "> ",
    [string]$Confirmation = "",
    [string]$Default = ""
  )
  $buffer = New-Object System.Collections.Generic.List[char];
  $buffer.AddRange($Default.ToCharArray())
  $cursor = $buffer.Count;
  $line = $Prompt + (-join $buffer);
  $NewBuf.SetLine($Row, ""); $Row++;
  $NewBuf.SetLine($Row, "#" * $NewBuf.Width);$Row++;
  $NewBuf.SetLine($Row, $Description);$Row++;
  $NewBuf.SetLine($Row, $line);
  $NewBuf.SetLine($Row+1, "#" * $NewBuf.Width);
  $NewBuf.SetLine($Row+2, "");

  try {
    Print-Diff -Old $OldBuf -New $NewBuf;
    [Console]::SetCursorPosition($Prompt.Length + $cursor, $Row);
    [Console]::CursorVisible = $true;

    while ($true) {
      $key = [Console]::ReadKey($true);

      if ( ($key.Modifiers -ceq "Alt" -and $key.KeyChar -ceq '0') -or ($key.Key -ceq 'Enter') -or ($key.Key -ceq 'F10') ) {
        if( [string]::IsNullOrWhiteSpace($Confirmation )){
          return -join $buffer;
        } else {
          $BeforeDialogBuf = [ScreenBuffer]::new($NewBuf);
          [Console]::CursorVisible = $false;
          $result = Show-ChooseDialog -OldBuf $OldBuf -NewBuf $NewBuf -Message $Confirmation;
              if ("Yes" -ceq $result) { return -join $buffer; }
          elseif ("No"  -ceq $result) { return $null; }
          elseif ("Cancel" -ceq $result)  {
            $NewBuf = $BeforeDialogBuf;
            Print-Diff -Old $OldBuf -New $NewBuf;
            [Console]::SetCursorPosition($Prompt.Length + $cursor, $Row);
            [Console]::CursorVisible = $true;
            continue;
          }
        }
      }
      switch -CaseSensitive ($key.Key) {
        'LeftArrow'  { $cursor = [Math]::Max(0, $cursor - 1); }
        'RightArrow' { $cursor = [Math]::Min($buffer.Count, $cursor + 1); }
        'Home'       { $cursor = 0; }
        'End'        { $cursor = $buffer.Count; }
        'Backspace' {
          if ($cursor -gt 0) {
            $cursor--;
            $buffer.RemoveAt($cursor);
          }
        }
        'Escape' { return $null; }
        'Delete' {
          if ($cursor -lt $buffer.Count) {
            $buffer.RemoveAt($cursor);
          }
        }
        default {
          if ($key.KeyChar -le 32) { break; }
          $buffer.Insert($cursor, $key.KeyChar);
          $cursor++;
        }
      }
      $line = $Prompt + (-join $buffer);
      $NewBuf.SetLine($Row, $line);
      Print-Diff -Old $OldBuf -New $NewBuf;
      [Console]::SetCursorPosition($Prompt.Length + $cursor, $Row);
    }
  } finally {
    [Console]::CursorVisible = $false;
  }
}

function Draw-UI {
  param(
    [Parameter(Mandatory)][ScreenBuffer]$buf,
    [string[]]$Header,
    $rows,
    [Parameter(Mandatory)][int]$selected,
    [Parameter(Mandatory)][int]$top,
    [string]$status
  )

  for ($i = 0; $i -lt $Header.Count; $i++) {
    $buf.SetLine($i, $Header[$i]);
  }

  $visibleHeight = $buf.Height - $Header.Count - 2;
  if ($visibleHeight -lt 1) { return; }

  $row = $Header.Count;
  $maxName = ($rows.Name | Measure-Object -Maximum Length).Maximum;
  if ($null -eq $maxName) { $maxName = 10; }
  $maxName = [Math]::Max($maxName, 10);
  $maxName = [Math]::Min($maxName, [Math]::Floor($buf.Width / 3));
  $maxType = ($rows.Name | Measure-Object -Maximum Length).Maximum;
  if ($null -eq $maxType) { $maxName = 10; }
  $maxType = [Math]::Max($maxType, 10);
  $maxType = [Math]::Min($maxType, [Math]::Floor($buf.Width / 3));
  $fmt = "{0,-2}{1,-$maxName} {2,-$maxType} {3}";

  $visibleRows = $rows | Select-Object -Skip $top -First $visibleHeight;
  $index = $top;

  foreach ($r in $visibleRows) {
    $marker = if ($index -eq $selected) { '>' } else { ' ' };
    $line   = $fmt -f $marker, $r.Name, $r.ValueKind, $r.Value;
    $buf.SetLine($row, $line);
    $row++;
    $index++;
  }

  while ($row -lt ($buf.Height - 2)) {
    $buf.SetLine($row, '');
    $row++;
  }
  $buf.SetLine($buf.Height - 2, "-" * $buf.Width);
  $buf.SetLine($buf.Height - 1, $status);
}

function Show-Help {
  clear;
  $helpText = @"
Registry TUI

A TUI program for navigating and editing the registry

Command -line parameters

 --path                   - provide a path in the form hive:/path/to/location (for example "HKCU:/Control Panel/Accessibility")
 --readonly               - disable all editing functionalities
 --help                   - show this dialog


TUI options

Arrow Keys                - move up and down
Home/End                  - go to start/end of list
Alt-0 / Q / F10 / Ctrl+C  - close program
Alt-1 / H / F1            - show this dialog
Alt-4 / E / F4            - edit registry value, create value if selected element is ..
Alt-5 / F5 / Ctrl+R       - refresh and redraw TUI
Alt-6 / F6                - rename registry key / value
Alt-7 / F7                - create registry key
Alt-8 / F8 / Canc         - delete registry key / value
N                         - next hive
G                         - go to location
Enter                     - open key

Press q to close.
"@

  $helpText | Out-Host -Paging;
  [Console]::ReadKey($true) | Out-Null;
}

$script:RegistryHives = @(
  [pscustomobject]@{ Name='HKLM'; Hive=[RegistryHive]::LocalMachine },
  [pscustomobject]@{ Name='HKCU'; Hive=[RegistryHive]::CurrentUser },
  [pscustomobject]@{ Name='HKCR'; Hive=[RegistryHive]::ClassesRoot },
  [pscustomobject]@{ Name='HKU';  Hive=[RegistryHive]::Users },
  [pscustomobject]@{ Name='HKCC'; Hive=[RegistryHive]::CurrentConfig }
)

function Get-RegistryKey {
  param(
    [Parameter(Mandatory)][int]$HiveIndex,
    [string]$Path,
    [bool]$Writable
  )

  $h = $script:RegistryHives[$HiveIndex];
  $base = [RegistryKey]::OpenBaseKey($h.Hive, [RegistryView]::Default);
  if ([string]::IsNullOrWhiteSpace($Path)) {
    return $base;
  }
  $sub = $base.OpenSubKey($Path, $Writable);
  $base.close();
  return $sub;
}

function Get-RegistryRows {
  param(
    [Parameter(Mandatory)][RegistryKey]$Key
  )
  $rows = [System.Collections.Generic.List[object]]::new();
  $rows.Add([pscustomobject]@{
    Type      = 'Up';
    Name      = '..';
    Value     = '';
    ValueKind = '';
  })

  if (-not $Key) { return ,$rows }

  foreach ($name in $Key.GetSubKeyNames()) {
    $rows.Add([pscustomobject]@{
      Type      = 'Key';
      Name      = $name;
      Value     = '';
      ValueKind = '';
    })
  }

  foreach ($name in $Key.GetValueNames()) {
    $val  = $Key.GetValue($name)
    $kind = $Key.GetValueKind($name)

    $disp = switch -CaseSensitive ($kind) {
      'Binary'      { "{0} bytes" -f $val.Length }
      'MultiString' { $val -join ", " }
      default       { [string]$val }
    }

    $rows.Add([pscustomobject]@{
      Type      = 'Value';
      Name      = $name;
      Value     = $disp;
      ValueKind = $kind;
    })
  }
  return ,$rows;
}

function Edit-RegistryValue {
  param(
    [Parameter(Mandatory)][ScreenBuffer]$OldBuf,
    [Parameter(Mandatory)][ScreenBuffer]$NewBuf,
    [Parameter(Mandatory)][RegistryKey]$Key,
    [string]$ValueName, # can be empty for default value
    [Parameter(Mandatory)][RegistryValueKind]$Kind
  )

  $current = $key.GetValue($ValueName);

  try {
    $new = Invoke-InlineEditor -OldBuf $OldBuf -NewBuf $NewBuf -Row ([Math]::Floor($NewBuf.Height / 3)) -Description "Insert value for '$ValueName'" -Prompt "> " -Default "$current" -Confirmation "Save changes?";

    if ( $null -eq $new ) { return "Cancelled"; }
    switch -CaseSensitive ($Kind) {
      'String'       { $key.SetValue($ValueName, $new, $Kind); }
      'Binary'       { $key.SetValue($ValueName, $new, $Kind); }
      'ExpandString' { $key.SetValue($ValueName, $new, $Kind); }
      'MultiString'  { $key.SetValue($ValueName, ($new -split ';'), $Kind); }
      'DWord'        { $key.SetValue($ValueName, [int]$new, $Kind); }
      'QWord'        { $key.SetValue($ValueName, [long]$new, $Kind); }
      default        { return "Unsupported type"; }
    }
    return "Updated";
  }
  catch {
    return $_.Exception.Message.Trim();
  }
}

function Invoke-CursesUI {
  param(
    [Parameter(Mandatory)][string]$path,
    [Parameter(Mandatory)][bool]$Writable
  )
  $raw = $Host.UI.RawUI

  $old = $null;
  $new = $null;

  $hiveIndex     = 1;
  if ($path -match '^[A-Za-z_]+[:\\/]') {
    $path = $path -replace '^HKEY_LOCAL_MACHINE' , 'HKLM';
    $path = $path -replace '^HKEY_CURRENT_USER'  , 'HKCU';
    $path = $path -replace '^HKEY_CLASSES_ROOT'  , 'HKCR';
    $path = $path -replace '^HKEY_USERS'         , 'HKU' ;
    $path = $path -replace '^HKEY_CURRENT_CONFIG', 'HKCC';
    $path = $path -replace '^HKLM[:\\/]', 'HKLM:';
    $path = $path -replace '^HKCU[:\\/]', 'HKCU:';
    $path = $path -replace '^HKCR[:\\/]', 'HKCR:';
    $path = $path -replace '^HKU[:\\/]' , 'HKU:' ;
    $path = $path -replace '^HKCC[:\\/]', 'HKCC:';
    $drive = (Split-Path $path -Qualifier).Split(':')[0];
    $hiveIndex = ([string[]]$script:RegistryHives.Name).IndexOf($drive);
    $path = (Split-Path $path -NoQualifier);
  }
  $path          = $path.Replace('/', '\').TrimStart('\').TrimEnd('\');
  $currentKey    = Get-RegistryKey -HiveIndex $hiveIndex -Path $path -Writable $Writable;
  $rows          = Get-RegistryRows -Key $currentKey;
  $selected      = 0;
  $top           = 0;
  $status        = "";
  $lastSize      = $raw.WindowSize;

  Init-Buffers -old ([ref]$old) -new ([ref]$new);

  try {
    [Console]::CursorVisible = $false;

    while ($true) {
      $windowSize=$raw.WindowSize
      # Handle console resize - does not work that well if windows is resized to same size
      # and does not redraw until user presses key
      if ($windowSize.Width -ne $lastSize.Width -or
          $windowSize.Height -ne $lastSize.Height) {

        Init-Buffers -old ([ref]$old) -new ([ref]$new);
        $lastSize = $windowSize
      }

      $hiveName = $script:RegistryHives[$hiveIndex].Name;
      $header = @(
        "Registry TUI",
        [string]("-" * [int]$new.Width),
        "${hiveName}:/$($path.Replace('\', '/'))",
        [string]("-" * [int]$new.Width)
      );

      $visibleHeight = [Math]::Max(1, $new.Height - $header.Count - 2)
      $top = [Math]::Min($top, $selected);
      $top = [Math]::Max($top, $selected - $visibleHeight + 1);
      $top = [Math]::Max(0, $top);

      Draw-UI -buf $new -Header $header -rows $rows -selected $selected -top $top -status $status;
      Print-Diff -Old $old -New $new;
      $status = "";

      $key = [Console]::ReadKey($true);

      switch -CaseSensitive ($key.Key) {

        "UpArrow" {
          if ( ($key.Modifiers -ceq "Shift" ) -and ($path -ne '')) { # NOTE: Alt would be better, but needs to use $raw.ReadKey("NoEcho, IncludeKeyDown") instead of [Console]::ReadKey
            $parts = $path -split '\\';
            $path = if ($parts.Count -gt 1) { $parts[0..($parts.Count-2)] -join '\' } else { '' };
            $currentKey.close();
            $currentKey = Get-RegistryKey -HiveIndex $hiveIndex -Path $path -Writable $Writable;
            $rows       = Get-RegistryRows -Key $currentKey;
            $selected   = 0;
          } else {
            if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $selected - 1);
          }
        }
        "DownArrow" { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Min($selected + 1, [Math]::Max(0, $rows.Count - 1));}
        "PageUp"    { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $selected - $visibleHeight); }
        "PageDown"  { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Min($selected + $visibleHeight, [Math]::Max(0, $rows.Count - 1));}
        "Home"      { if ($key.Modifiers -ne 0) { break; } $selected = 0; }
        "End"       { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $rows.Count - 1);}

        "Enter" {
          if ($rows.Count -eq 0) { break; }
          if ($key.Modifiers -ne 0) { break; }
          $item = $rows[$selected];

          if ($item.Type -ceq 'Up' -and $path -ne '') {
            $parts = $path -split '\\';
            $parent = if ($parts.Count -gt 1) { $parts[0..($parts.Count-2)] -join '\' } else { '' };
            try {
              $currentKey2 = Get-RegistryKey -HiveIndex $hiveIndex -Path $parent -Writable $Writable;
              $currentKey.close();
              $currentKey = $currentKey2;
              $path       = $parent;
              $rows       = Get-RegistryRows -Key $currentKey;
              $selected   = 0;
            } catch {
              $status = "Unable to open parent Key";
            }
          } elseif ($item.Type -ceq 'Up' -and $path -ceq '') {
            # use to choose hive instead o N
          } elseif ($item.Type -ceq 'Key') {
            $sub = if ($path -ceq '') { $item.Name } else { "$path\$($item.Name)" };
            try {
              $currentKey2 = Get-RegistryKey -HiveIndex $hiveIndex -Path $sub -Writable $Writable;
              $currentKey.close();
              $currentKey = $currentKey2;
              $path       = $sub;
              $rows       = Get-RegistryRows -Key $currentKey;
              $selected   = 0;
            } catch {
              $status = "Unable to open Key";
            }
          }
        }

        "F4" { if ($key.Modifiers -ne 0) { break; } $_ = "E"} 
        "D4" { if( $key.Modifiers -cne "Alt" ) { break ;}  $_ = "E"} 
        "E" {
          if ($key.Modifiers -ne 0) { break; }
          if (-not $Writable) { $status = "Edit not supported in read-only mode"; break; }
          if ($rows.Count -eq 0) { break; }
          $item = $rows[$selected];
          if ($item.Type -ceq 'Value') {
            $status = Edit-RegistryValue -OldBuf $old -NewBuf $new -Key $currentKey -ValueName $item.Name -Kind $item.ValueKind;
            $rows   = Get-RegistryRows -Key $currentKey;
          }
        }

        "R"  { if( $key.Modifiers -cne "Control" ) { break ;} $_ = "F5"; }
        "D5" { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "F5"; }
        "F5" {
          Init-Buffers -old ([ref]$old) -new ([ref]$new);
          $currentKey.close();
          $currentKey = Get-RegistryKey -HiveIndex $hiveIndex -Path $path -Writable $Writable;
          $rows       = Get-RegistryRows -Key $currentKey;
          $selected   = 0;
          $status     = "Registry hive reloaded";
        }

        "D6" { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "F6"; }
        "F6" {
          if (-not $Writable) { $status = "Rename not supported in read-only mode"; break; }
          if ($rows.Count -eq 0) { break; }
          $item = $rows[$selected];
          if ($item.Type -ceq 'Value') {
            # note: use PowerShell instead of .Net API as I found no function for renaming
            try {
              $fullPath = "${hiveName}:\$path";
              $description = "Write new Key name (empty are ignored)";
              $newName = Invoke-InlineEditor -OldBuf $old -NewBuf $new -Row ([Math]::Floor($new.Height / 3)) -Description $description -Prompt "> " -Default $item.Name;
              if ([string]::IsNullOrWhiteSpace($newName) -or $item.Name -ceq $newName) {
                $status = "Value rename cancelled";
                break;
              }
              Rename-ItemProperty -Path $fullPath -Name $item.Name -NewName $newName -ErrorAction Stop; # FIXME use $currentKey API
              $status = "renamed key '$($item.Name)' to '$newName'";
              $rows   = Get-RegistryRows -Key $currentKey;
            } catch {
              $status = "Failed to rename value '$newName': $_".Trim();
            }
          } elseif ($item.Type -ceq 'Key') {
            try {
              $fullPath = "${hiveName}:\$path\$($item.Name)";
              $description = "Write new Key name (empty are ignored)";
              $newName = Invoke-InlineEditor -OldBuf $old -NewBuf $new -Row ([Math]::Floor($new.Height / 3)) -Description $description -Prompt "> " -Default $item.Name;
              Init-Buffers -old ([ref]$old) -new ([ref]$new);
              if ([string]::IsNullOrWhiteSpace($newName) -or $item.Name -ceq $newName) {
                $status = "Key rename cancelled";
                break;
              }
              Copy-Item -Path $fullPath -Destination "${hiveName}:\$path\$newName" -Recurse -Force -ErrorAction Stop; # FIXME use $currentKey API
              Remove-Item -Path $fullPath -Recurse -Force -ErrorAction Stop;
              $status = "renamed key '$($item.Name)' to '$newName'";
              $rows   = Get-RegistryRows -Key $currentKey;
            } catch {
              $status = "Failed to rename key '$newName': $_".Trim();
            }
          }
        }

        "D7" { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "F7"; }
        "F7" {
          if (-not $Writable) { $status = "Key/Value creation not supported in read-only mode"; break; }
          $type = Show-ChooseDialog -OldBuf $old -NewBuf $new -Message "Choose what key/value to create" -Buttons @("Key",
           "Value(String)", "Value(DWord)", "Value(QWord)", "Value(Binary)", "Value(MultiString)", "Value(ExpandString)",
           "Key(Volatile)",
           "Cancel"
          );
          # NOTE: missing optiona for making volatile key
          if( $type -ceq "Cancel"){ $status = "Creation of Key/value cancelled"; break;}

          if( ($type -ceq "Key") -or ($type -ceq "Key(Volatile)")) {
            $description = "Insert Key name to create (can't be empty)";
            $confirmation = "Create Key?";
            $status1 = "Key creation cancelled";
          } else {
            $description = "Insert Value name to create (leave empty for default value)";
            $confirmation = "";
            $status1 = "Value creation cancelled";
          }

          $newName = Invoke-InlineEditor -OldBuf $old -NewBuf $new -Row ([Math]::Floor($new.Height / 3)) -Description $description -Prompt "> " -Confirmation $confirmation;
          if ( $null -eq $newName ) {
            $status = "Key creation cancelled";
            break;
          }

          if( ($type -ceq "Key") -or ($type -ceq "Key(Volatile)")) {
            if ( $newName -ceq '' ) { break; }
            # FIXME: if already exist, change status and break, but no error
            try {
              if( $type -ceq "Key(Volatile)" ) {
                ($currentKey.CreateSubKey($newName, $true, "Volatile")).Close();
              } else {
                ($currentKey.CreateSubKey($newName)).Close();
              }
              $rows   = Get-RegistryRows -Key $currentKey;
              $status = "Created key '$newName'";
            } catch {
              $status = "Failed to create key '$newName': $_".Trim();
            }
          } elseif ($type -match '^Value\((.+)\)$') {
            $type=$Matches[1];
            try {
              $status = Edit-RegistryValue -OldBuf $old -NewBuf $new -Key $currentKey -ValueName $newName -Kind $type;
              $rows   = Get-RegistryRows -Key $currentKey;
              $status = "Created value '$newName'";
            } catch {
              $status = "Failed to create value of type string '$newName': $_".Trim();
            }
          } else {
            $status= "Unexpected type: '$type'";
          }
        }

        "D8"     { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "F8"; }
        "Delete" { if( $key.Modifiers -ne 0 ) { break; } $_ = "F8"; }
        "F8" {
          if ($rows.Count -eq 0) { break; }
          if (-not $Writable) { $status = "Key deletion not supported in read-only mode"; break; }
          $item = $rows[$selected];
          if ($item.Type -ceq 'Up') {
            $status = "Unable to remove parent key";
            break;
          }
          $resp = Show-ChooseDialog -OldBuf $old -NewBuf $new -Message "Do you want to remove '$($item.Name)'?"
          if( ! ($resp -ceq "Yes") )  { break;}
          if ($item.Type -ceq 'Value') {
            $fullPath = "${hiveName}:\$path";
            try {
              $currentKey.DeleteValue($item.Name, $false);
              $status   = "Removed Value '$($item.Name)'";
              $rows     = Get-RegistryRows -Key $currentKey;
              $selected = [Math]::Min($selected, [Math]::Max(0, $rows.Count - 1));
            } catch {
              $status = "Failed to remove value '$($item.Name)': $_".Trim();
            }
          } elseif ($item.Type -ceq 'Key') {
            try {
              $currentKey.DeleteSubKeyTree($item.Name);
              $status   = "Removed key '$($item.Name)'";
              $rows     = Get-RegistryRows -Key $currentKey;
              $selected = [Math]::Min($selected, [Math]::Max(0, $rows.Count - 1));
            } catch {
              $status = "Failed to remove key '$($item.Name)': $_".Trim();
            }
          }
        }

        "G" { # G -> goto
          $newpath = Invoke-InlineEditor -OldBuf $old -NewBuf $new -Row ([Math]::Floor($new.Height / 3)) -Description "Insert Key to go" -Prompt "Goto: " -Default "Environment" -Confirmation "Go to selected Key?";
          if ( [string]::IsNullOrWhiteSpace($newpath) ) { break; }
          $newpath = $newpath.Trim().Replace('/', '\').TrimStart('\').TrimEnd('\');
          if( $newpath -ceq $path ) { break; }
          try {
            $currentKey.close();
            $currentKey = Get-RegistryKey -HiveIndex $hiveIndex -Path $newpath -Writable $Writable;
            $rows       = Get-RegistryRows -Key $currentKey;
            $path       = $newpath;
            $status     = "path:$path";
            $selected   = 0;
          } catch {
            $status = "Unable to go to '${newpath}': $_".Trim()
          }
        }

        "N" { # Next Hive
          if( ($key.Modifiers -cne "Alt") -and ($key.Modifiers -cne "") ) { break; }
          $hiveIndex  = ($hiveIndex + 1) % $script:RegistryHives.Count;
          $path       = '';
          $currentKey.close();
          $currentKey = Get-RegistryKey -HiveIndex $hiveIndex -Path $path -Writable $Writable;
          $rows       = Get-RegistryRows -Key $currentKey;
          $selected   = 0;
          $status     = "Switched hive";
        }

        "D1"  { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "H"; }
        "F1"  { if ($key.Modifiers -ne 0) { break; } $_ = "H" }
        "H"   {
          Show-Help;
          Init-Buffers -old ([ref]$old) -new ([ref]$new);
          # keep current position
        }

        "D0"  { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "Q"; } # Alt+0
        "F10" { if ($key.Modifiers -ne 0) { break; } $_ = "Q"; } # F10 = Quit
        "Q"   { return; } # Q

        default {
          if( $key.KeyChar -ceq "?"){
            Show-Help;
            Init-Buffers -old ([ref]$old) -new ([ref]$new);
            break;
          }
          #$status = "$($key.Key), $($key.KeyChar), $($key.Modifiers)";
        }
      }
    }
  } finally {
    [Console]::CursorVisible = $true;
    clear;
  }
}



$helpFlags = @("--help", "-h", "/?", "?", "-help");
$PathValue="HKCU:/";
$Writable=$true;
for ($i = 0; $i -lt $Args.Count; $i++) {
  $arg = $Args[$i]
  if ($helpFlags -ccontains $arg) {
    Show-Help;
    exit 0;
  }
  switch -CaseSensitive ($arg) {
    "--path" {
      if ($i + 1 -ge $Args.Count) {
        [Console]::Error.WriteLine("Missing required value after --path");
        exit 1;
      }
      $PathValue = $Args[$i + 1];
      $i++;
    }
    "--readonly" {$Writable=$false;}
    default {
      [Console]::Error.WriteLine("Unrecognized parameter: $arg");
      exit 1;
    }
  }
}

Invoke-CursesUI -Path $PathValue -Writable $Writable;
