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

  $raw = $Host.UI.RawUI;

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
      $chars = [string]::new($rowNew, $start, $length);
      [Console]::Write( $chars );
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
    [Console]::CursorVisible = $true;
    Print-Diff -Old $OldBuf -New $NewBuf;
    [Console]::SetCursorPosition($Prompt.Length + $cursor, $Row);

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
  $maxName = ($rows.DisplayName | Measure-Object -Maximum Length).Maximum;
  if ($null -eq $maxName) { $maxName = 10; }
  $maxName = [Math]::Max($maxName, 10);
  $maxName = [Math]::Min($maxName, [Math]::Floor($buf.Width / 3));
  $fmt = "{0,-2}{1,-$maxName} {2,-10} {3,-9} {4,-5} {5}";

  $visibleRows = $rows | Select-Object -Skip $top -First $visibleHeight;
  $index = $top;

  foreach ($r in $visibleRows) {
    $marker = if ($index -eq $selected) { '>' } else { ' ' };
    $state = if ($r.Enabled -eq "True") { "Enabled" } else { "Disabled" };
    $line   = $fmt -f $marker, $r.DisplayName, $state, $r.Direction, $r.Profile, $r.Action;
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
Firewall TUI

A TUI program for listing, enabling, and disabling firewall rules

Command -line parameters

 --help                   - show this dialog


TUI options

Arrow Keys                - move up and down
Home/End                  - go to start/end of list
Alt-0 / Q / F10 / Ctrl+C  - close program
Alt-1 / H / F1            - show this dialog
Alt-5 / F5 / Ctrl+R       - refresh and redraw TUI
Ctrl+F / F                - filter rules
Space                     - enable/disable firewall rule

Press q to close.
"@

  $helpText | Out-Host -Paging;
  [Console]::ReadKey($true) | Out-Null;
}

function Get-FwRules {
    return @(Get-NetFirewallRule | Sort-Object Profile, DisplayName )
}

function Invoke-CursesUI {
  $raw = $Host.UI.RawUI

  $old = $null;
  $new = $null;

  [Console]::WriteLine( "Loading firewall rules..." );
  $allrows       = Get-FwRules;
  $filter        = "";
  $rows          = @($allrows | Where-Object { $_.DisplayName -match $filter });
  $selected      = 0;
  $top           = 0;
  $status        = "Loaded $($rows.Count) rules";
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

      $header = @(
        "Firewall TUI",
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

        "UpArrow"   { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $selected - 1);}
        "DownArrow" { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Min($selected + 1, [Math]::Max(0, $rows.Count - 1));}
        "PageUp"    { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $selected - $visibleHeight); }
        "PageDown"  { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Min($selected + $visibleHeight, [Math]::Max(0, $rows.Count - 1));}
        "Home"      { if ($key.Modifiers -ne 0) { break; } $selected = 0; }
        "End"       { if ($key.Modifiers -ne 0) { break; } $selected = [Math]::Max(0, $rows.Count - 1);}

        "R"  { if( $key.Modifiers -cne "Control" ) { break ;} $_ = "F5"; }
        "D5" { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "F5"; }
        "F5" {
          Init-Buffers -old ([ref]$old) -new ([ref]$new);
          $allrows  = Get-FwRules;
          $rows     = @($allrows | Where-Object { $_.TaskName -match $filter });
          $selected = 0;
          $status   = "Loaded $($rows.Count) firewall rules";
        }

        "Spacebar" { # Space -> toggle status
          if ($key.Modifiers -ne 0) { break; }
          if ($rows.Count -eq 0) { break; }
          $item = $rows[$selected];
          try {
            $status = if ( $item.Enabled ) { "Disabling $($item.DisplayName)..." } else {"Enabling $($item.DisplayName)..." };
            Draw-UI -buf $new -Header $header -rows $rows -selected $selected -top $top -status $status;
            Print-Diff -Old $old -New $new;
            if ( $item.Enabled -eq "True" ) {
	      Disable-NetFirewallRule -Name $item.Name -ErrorAction Stop | Out-Null;
              $status = "Disabled $($item.DisplayName)";
            } else {
	      Enable-NetFirewallRule -Name $item.Name -ErrorAction Stop | Out-Null;
              $status = "Enabled $($item.DisplayName)";
            }
            $allrows     = Get-FwRules;
            $rows        = @($allrows | Where-Object { $_.DisplayName -match $filter });
          } catch {
            $status = "Failed to modify rule '$($item.DisplayName)': $_".Trim();
          }
        }

        "D1"  { if( $key.Modifiers -cne "Alt" ) { break ;} $_ = "H"; }
        "F1"  { if ($key.Modifiers -ne 0) { break; } $_ = "H" }
        "H"   {
          Show-Help;
          Init-Buffers -old ([ref]$old) -new ([ref]$new);
          # keep current position
        }

        "F"  {
          if( ($key.Modifiers -cne "Control") -and ($key.Modifiers -ne 0) ) { break; }
          $newFilter = Invoke-InlineEditor -OldBuf $old -NewBuf $new -Row ([Math]::Floor($new.Height / 3)) -Description "Write filter, leave empty to show all tasks, press Esc to cancel" -Prompt "Filter: " -Default $filter;
          if( ($null -eq $newFilter) -or ($filter -eq $newFilter) ) { break; }
          $filter = $newFilter;
          $rows = @($allrows | Where-Object { $_.DisplayName -match $filter });
          $status="Filter '$filter' applied, showing $($rows.Count) rules";
          $selected = 0;
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
for ($i = 0; $i -lt $Args.Count; $i++) {
  $arg = $Args[$i]
  if ($helpFlags -ccontains $arg) {
    Show-Help;
    exit 0;
  }
  [Console]::Error.WriteLine("Unrecognized parameter: $arg");
  exit 1;
}

Invoke-CursesUI;
