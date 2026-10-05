# ui-drive.ps1 вЂ” РїСЂРѕРІРµСЂРєР° РѕРєРЅР° В«РєР°Рє РїРѕР»СЊР·РѕРІР°С‚РµР»СЊВ» Рё СЃРЅРёРјРєРё РґР»СЏ РѕС‚С‡С‘С‚Р°.
#
# Р—Р°С‡РµРј СЌС‚РѕС‚ СЃРєСЂРёРїС‚ РІ СЂРµРїРѕР·РёС‚РѕСЂРёРё: РёРЅС‚РµСЂС„РµР№СЃ РЅСѓР¶РЅРѕ РїСЂРѕРІРµСЂСЏС‚СЊ С‚РµРјРё Р¶Рµ СЃРїРѕСЃРѕР±Р°РјРё,
# РєР°РєРёРјРё РµРіРѕ С‚СЂРѕРіР°РµС‚ С‡РµР»РѕРІРµРє. РЎРѕРѕР±С‰РµРЅРёСЏ СЃ СѓРєР°Р·Р°С‚РµР»РµРј (WM_NOTIFY СЃ СЃР°РјРѕРґРµР»СЊРЅС‹Рј
# NMHDR, TCM_GETITEMRECT, LVM_GETITEMTEXTвЂ¦) РёР· РґСЂСѓРіРѕРіРѕ РїСЂРѕС†РµСЃСЃР° СЂРѕРЅСЏСЋС‚ comctl32 РІ
# С†РµР»РµРІРѕРј РѕРєРЅРµ, РїРѕС‚РѕРјСѓ С‡С‚Рѕ Windows РЅРµ РїРµСЂРµРЅРѕСЃРёС‚ СѓРєР°Р·Р°С‚РµР»Рё РјРµР¶РґСѓ РїСЂРѕС†РµСЃСЃР°РјРё вЂ” С‚Р°РєРёРµ
# В«РґРµС„РµРєС‚С‹В» РїРѕС‚РѕРј РёС‰СѓС‚ РІ РїСЂРёР»РѕР¶РµРЅРёРё, Р° РёС… С‚Р°Рј РЅРµС‚. Р—РґРµСЃСЊ РІСЃС‘ Р±РµР·РѕРїР°СЃРЅРѕ:
# WM_COMMAND (С‡РёСЃР»Р°), РІС‹Р±РѕСЂ СЃС‚СЂРѕРєРё РІ СЃРїРёСЃРєРµ Рё С‰РµР»С‡РєРё РјС‹С€СЊСЋ.
#
# РџСЂРёРјРµСЂС‹:
#   powershell -File rust\tools\ui-drive.ps1 -Action main           -Out shot.png
#   powershell -File rust\tools\ui-drive.ps1 -Action filter-app     -Out shot.png
#   powershell -File rust\tools\ui-drive.ps1 -Action detail-full    -Out shot.png
#   powershell -File rust\tools\ui-drive.ps1 -Action multi-select   -Out shot.png
#   powershell -File rust\tools\ui-drive.ps1 -Action purge          -Out shot.png
#   powershell -File rust\tools\ui-drive.ps1 -Action settings       -Out shot.png
param(
    [ValidateSet('main', 'filter-app', 'filter-reset', 'detail', 'detail-full',
                 'font', 'context', 'multi-select', 'delete-selected', 'purge',
                 'purge-delete', 'file-menu', 'settings', 'hotkeys',
                 'simple-view', 'theme-dark', 'theme-light', 'feed-time-column',
                 'feed-three-lines', 'confirm-delete', 'quit')]
    [string]$Action = 'main',
    [string]$Out = "$env:TEMP\wnl-shot.png",
    [string]$Exe = "$PSScriptRoot\..\dist\wnl.exe"
)

Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class UiDrive {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder b, int n);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr h, int id);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder b, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static IntPtr Found = IntPtr.Zero;
    public static string Wanted = "";
    public static uint Pid = 0;
    public static bool Callback(IntPtr h, IntPtr l) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        if (pid != Pid) return true;
        var sb = new StringBuilder(256); GetClassName(h, sb, sb.Capacity);
        if (sb.ToString() == Wanted) { Found = h; return false; }
        return true;
    }
    public static IntPtr Find(uint pid, string cls) {
        Pid = pid; Wanted = cls; Found = IntPtr.Zero;
        EnumWindows(new EnumProc(Callback), IntPtr.Zero);
        return Found;
    }
    public static void SelectCombo(IntPtr combo, int index) { SendMessage(combo, 0x014E, (IntPtr)index, IntPtr.Zero); }
    // WM_COMMAND: code 1 is CBN_SELCHANGE for a list and BN_CLICKED for a button.
    public static void Command(IntPtr window, int id, int code) {
        SendMessage(window, 0x0111, (IntPtr)((code << 16) | (id & 0xFFFF)), IntPtr.Zero);
    }
    // A minimised window has to be shown again first: the mouse clicks and the
    // screen copies below need it where it says it is.
    public static void Bring(IntPtr window) {
        ShowWindow(window, 9);      // SW_RESTORE
        SetForegroundWindow(window);
        System.Threading.Thread.Sleep(400);
    }
    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(120);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
    }
    public static void Escape() {
        keybd_event(0x1B, 0, 0, UIntPtr.Zero);
        keybd_event(0x1B, 0, 2, UIntPtr.Zero);
    }
    // Modifiers and the right button: a multiple selection is made the same way a
    // person makes it - click, Ctrl+click, then the context menu.
    public static void KeyDown(byte vk) { keybd_event(vk, 0, 0, UIntPtr.Zero); }
    public static void KeyUp(byte vk) { keybd_event(vk, 0, 2, UIntPtr.Zero); }
    // A character is posted as WM_CHAR: no pointer crosses the process boundary,
    // so this is exactly as safe as a key press.
    public static void TypeText(IntPtr edit, string text) {
        foreach (char character in text) {
            PostMessage(edit, 0x0102, (IntPtr)character, IntPtr.Zero);
            System.Threading.Thread.Sleep(40);
        }
    }
    public static void RightClick(int x, int y) {
        SetCursorPos(x, y);
        System.Threading.Thread.Sleep(120);
        mouse_event(0x0008, 0, 0, 0, UIntPtr.Zero);
        System.Threading.Thread.Sleep(60);
        mouse_event(0x0010, 0, 0, 0, UIntPtr.Zero);
    }
}

'@

function Get-AppProcess {
    return Get-Process wnl -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
}

function Get-AppWindow([string]$Class) {
    $process = Get-AppProcess
    if (-not $process) { return [IntPtr]::Zero }
    return [UiDrive]::Find([uint32]$process.Id, $Class)
}

function Get-WindowSize([IntPtr]$Window) {
    $rect = New-Object UiDrive+RECT
    [UiDrive]::GetWindowRect($Window, [ref]$rect) | Out-Null
    return ('{0}x{1}' -f ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top))
}

function Save-Window([IntPtr]$Window, [string]$Path, [switch]$FromScreen) {
    if ($Window -eq [IntPtr]::Zero) { Write-Output 'window not found'; return }
    $rect = New-Object UiDrive+RECT
    [UiDrive]::GetWindowRect($Window, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    if ($FromScreen) {
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
    } else {
        $dc = $graphics.GetHdc()
        $printed = [UiDrive]::PrintWindow($Window, $dc, 2)
        $graphics.ReleaseHdc($dc)
        if (-not $printed) { $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size) }
    }
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Write-Output "saved $Path ($width x $height)"
}

# A pop-up menu is drawn by the system on the desktop, so it only appears on a
# screen copy of the window area.
function Save-Screen([IntPtr]$Window, [string]$Path) {
    $rect = New-Object UiDrive+RECT
    [UiDrive]::GetWindowRect($Window, [ref]$rect) | Out-Null
    $left = [Math]::Max(0, $rect.Left - 20)
    $top = [Math]::Max(0, $rect.Top - 20)
    $width = [Math]::Min(2560, $rect.Right - $left + 20)
    $height = [Math]::Min(1440, $rect.Bottom - $top + 20)
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    Write-Output "saved screen $Path ($width x $height)"
}

$main = Get-AppWindow 'WinNotificationLogNative'
if ($main -eq [IntPtr]::Zero) {
    Write-Output "the window is not running - start $Exe first"
    exit 1
}

switch ($Action) {
    'main' { Save-Window $main $Out }

    'filter-app' {
        # Application list: entry 0 is "Р’СЃРµ РїСЂРёР»РѕР¶РµРЅРёСЏ", entry 1 the first real one.
        $combo = [UiDrive]::GetDlgItem($main, 114)
        [UiDrive]::SelectCombo($combo, 1)
        [UiDrive]::Command($main, 114, 1)
        Start-Sleep -Milliseconds 900
        Save-Window $main $Out
    }

    'filter-reset' {
        [UiDrive]::Command($main, 120, 0)   # В«РЎР±СЂРѕСЃРёС‚СЊВ»
        Start-Sleep -Milliseconds 900
        Save-Window $main $Out
    }

    'detail' {
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]212, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1500
        Save-Window (Get-AppWindow 'WinNotificationLogDetail') $Out
    }

    'detail-full' {
        $detail = Get-AppWindow 'WinNotificationLogDetail'
        if ($detail -eq [IntPtr]::Zero) {
            [UiDrive]::PostMessage($main, 0x0111, [IntPtr]212, [IntPtr]::Zero) | Out-Null
            Start-Sleep -Milliseconds 1500
            $detail = Get-AppWindow 'WinNotificationLogDetail'
        }
        # The second page is clicked, exactly like a user would.
        $tab = [UiDrive]::GetDlgItem($detail, 406)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($tab, [ref]$rect) | Out-Null
        [UiDrive]::SetForegroundWindow($detail) | Out-Null
        Start-Sleep -Milliseconds 300
        [UiDrive]::Click($rect.Left + 128, $rect.Top + 15)
        Start-Sleep -Milliseconds 700
        Save-Window $detail $Out
    }

    'font' {
        # 150 %: the size users reach for first on a big screen.
        [UiDrive]::Bring($main)
        [UiDrive]::Command($main, 233, 0)
        Start-Sleep -Milliseconds 1400
        Save-Window $main $Out
    }

    'context' {
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $list = [UiDrive]::GetDlgItem($main, 109)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($list, [ref]$rect) | Out-Null
        $y = $rect.Top + 120
        $x = $rect.Left + 200
        $lparam = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
        [UiDrive]::PostMessage($main, 0x007B, [IntPtr]$list, $lparam) | Out-Null
        Start-Sleep -Milliseconds 900
        Save-Screen $main $Out
        [UiDrive]::Escape()
        Start-Sleep -Milliseconds 300
    }

    'multi-select' {
        # Click a row, Ctrl+click two more: the menu has to act on all three.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $list = [UiDrive]::GetDlgItem($main, 109)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($list, [ref]$rect) | Out-Null
        $x = $rect.Left + 200
        $first = $rect.Top + 46          # below the column header
        $row = 22                        # row height at 100 %
        [UiDrive]::Click($x, $first)
        Start-Sleep -Milliseconds 250
        [UiDrive]::KeyDown(0x11)         # Ctrl
        [UiDrive]::Click($x, $first + $row)
        Start-Sleep -Milliseconds 250
        [UiDrive]::Click($x, $first + 2 * $row)
        Start-Sleep -Milliseconds 250
        [UiDrive]::KeyUp(0x11)
        Start-Sleep -Milliseconds 250
        # Right click on one of the selected rows: the whole selection is meant.
        [UiDrive]::RightClick($x, $first + 2 * $row)
        Start-Sleep -Milliseconds 900
        Save-Screen $main $Out
        [UiDrive]::Escape()
        Start-Sleep -Milliseconds 300
    }

    'purge' {
        # The period window: the date is typed and the count is read from the window.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]216, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1200
        $purge = Get-AppWindow 'WinNotificationLogPurge'
        if ($purge -ne [IntPtr]::Zero) {
            [UiDrive]::TypeText([UiDrive]::GetDlgItem($purge, 603), '01.01.2020')
            Start-Sleep -Milliseconds 1500
        }
        Save-Window $purge $Out -FromScreen
        # Close it without deleting anything: 608 is its "Закрыть" button.
        [UiDrive]::PostMessage($purge, 0x0111, [IntPtr]608, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 400
    }

    'delete-selected' {
        # Полный путь удаления: выбрать строки → команда → подтверждение → удаление.
        # Запускать с --home во временной папке: журнал действительно теряет записи.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $list = [UiDrive]::GetDlgItem($main, 109)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($list, [ref]$rect) | Out-Null
        $x = $rect.Left + 200
        $first = $rect.Top + 46
        $row = 22
        [UiDrive]::Click($x, $first)
        Start-Sleep -Milliseconds 250
        [UiDrive]::KeyDown(0x11)
        [UiDrive]::Click($x, $first + $row)
        Start-Sleep -Milliseconds 250
        [UiDrive]::KeyUp(0x11)
        Start-Sleep -Milliseconds 250
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]214, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1200
        # Вопрос «Удалить выбранные?»: жмём «Да» (IDYES) как пользователь.
        $question = [UiDrive]::Find([uint32](Get-AppProcess).Id, '#32770')
        $yes = [UiDrive]::GetDlgItem($question, 6)
        [UiDrive]::SendMessage($yes, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1500
        Save-Window $main $Out
    }

    'purge-delete' {
        # Удаление за период целиком: открыть окно, ввести дату, нажать «Удалить…»,
        # подтвердить. Запускать с --home во временной папке: записи исчезнут.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]216, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1200
        $purge = Get-AppWindow 'WinNotificationLogPurge'
        [UiDrive]::TypeText([UiDrive]::GetDlgItem($purge, 603), '01.01.2020')
        Start-Sleep -Milliseconds 1400
        Save-Window $purge "$Out-before" -FromScreen
        [UiDrive]::PostMessage($purge, 0x0111, [IntPtr]607, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1200
        $question = [UiDrive]::Find([uint32](Get-AppProcess).Id, '#32770')
        $yes = [UiDrive]::GetDlgItem($question, 6)
        [UiDrive]::SendMessage($yes, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1800
        Save-Window $main $Out
    }

    'file-menu' {
        # Меню «Файл»: команды удаления должны быть видны и иметь правильные подписи.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($main, [ref]$rect) | Out-Null
        [UiDrive]::Click($rect.Left + 38, $rect.Top + 50)
        Start-Sleep -Milliseconds 800
        Save-Screen $main $Out
        [UiDrive]::Escape()
        Start-Sleep -Milliseconds 300
    }

    'hotkeys' {
        # Ctrl+A and Ctrl+C in the list and in the search field.  The proof is the
        # clipboard: what Ctrl+C put there is what Ctrl+A had selected.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $search = [UiDrive]::GetDlgItem($main, 102)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($search, [ref]$rect) | Out-Null
        [UiDrive]::Click($rect.Left + 40, [int](($rect.Top + $rect.Bottom) / 2))
        Start-Sleep -Milliseconds 300
        [UiDrive]::TypeText($search, 'wnl-test')
        Start-Sleep -Milliseconds 600
        # Ctrl+A (select all), Ctrl+C (copy), then read the clipboard back.
        Set-Clipboard -Value 'пусто'
        [UiDrive]::KeyDown(0x11)
        [UiDrive]::KeyDown(0x41); [UiDrive]::KeyUp(0x41)
        [UiDrive]::KeyDown(0x43); [UiDrive]::KeyUp(0x43)
        [UiDrive]::KeyUp(0x11)
        Start-Sleep -Milliseconds 600
        Write-Output ("search field, ctrl+a then ctrl+c: '{0}'" -f (Get-Clipboard))
        # Escape clears the field; its text should be empty afterwards.
        [UiDrive]::Escape()
        # The cleared search reloads the list a moment later; wait for that before
        # touching the list, otherwise the reload wipes the selection again.
        Start-Sleep -Milliseconds 1500
        $buffer = New-Object System.Text.StringBuilder 256
        [UiDrive]::GetWindowText($search, $buffer, 256) | Out-Null
        Write-Output ("after escape the search field holds: '{0}'" -f $buffer.ToString())
        # The list: select everything with Ctrl+A and copy it.
        $list = [UiDrive]::GetDlgItem($main, 109)
        [UiDrive]::GetWindowRect($list, [ref]$rect) | Out-Null
        [UiDrive]::Click($rect.Left + 200, $rect.Top + 46)
        Start-Sleep -Milliseconds 300
        [UiDrive]::KeyDown(0x11)
        [UiDrive]::KeyDown(0x41); [UiDrive]::KeyUp(0x41)
        [UiDrive]::KeyUp(0x11)
        # Measured right away and then a few times: a reload of the list (the
        # collector, the search debounce) rebuilds the rows and selects one again,
        # which is worth seeing rather than guessing.
        foreach ($wait in 0, 200, 400, 800) {
            if ($wait -gt 0) { Start-Sleep -Milliseconds $wait }
            $count = [UiDrive]::SendMessage($list, 0x1032, [IntPtr]::Zero, [IntPtr]::Zero)
            Write-Output ("  через {0,4} мс после Ctrl+A выбрано: {1}" -f $wait, $count)
        }
        [UiDrive]::KeyDown(0x11)
        [UiDrive]::KeyDown(0x43); [UiDrive]::KeyUp(0x43)
        [UiDrive]::KeyUp(0x11)
        Start-Sleep -Milliseconds 900
        $rows = [UiDrive]::SendMessage($list, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero)
        $selected = [UiDrive]::SendMessage($list, 0x1032, [IntPtr]::Zero, [IntPtr]::Zero)
        $copied = (Get-Clipboard) -join "`n"
        $titles = ([regex]::Matches($copied, 'Проверка журнала №\d')).Count
        Write-Output ("list: строк {0}, выбрано {1}, заголовков в буфере {2}" -f $rows, $selected, $titles)
        Save-Window $main $Out
    }

    'confirm-delete' {
        # Enter must confirm the deletion: the focus of the question sits on
        # «Удалить», not on «Отмена».  Run with --home in a temporary folder: this
        # really deletes records.
        [UiDrive]::Bring($main)
        Start-Sleep -Milliseconds 300
        $list = [UiDrive]::GetDlgItem($main, 109)
        $before = [UiDrive]::SendMessage($list, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero)
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($list, [ref]$rect) | Out-Null
        [UiDrive]::Click($rect.Left + 200, $rect.Top + 46)
        Start-Sleep -Milliseconds 400
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]214, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1200
        $question = [UiDrive]::Find([uint32](Get-AppProcess).Id, 'WinNotificationLogConfirm')
        if ($question -eq [IntPtr]::Zero) { Write-Output 'the question window did not appear'; return }
        $yes = [UiDrive]::GetDlgItem($question, 902)
        $no = [UiDrive]::GetDlgItem($question, 903)
        foreach ($pair in @(@('подтверждение', $yes), @('отмена', $no))) {
            $buffer = New-Object System.Text.StringBuilder 128
            [UiDrive]::GetWindowText($pair[1], $buffer, 128) | Out-Null
            Write-Output ("кнопка {0}: '{1}'" -f $pair[0], $buffer.ToString())
        }
        Save-Window $question "$Out-question" -FromScreen
        # Enter, as a user would press it after reading the question.
        [UiDrive]::KeyDown(0x0D); [UiDrive]::KeyUp(0x0D)
        Start-Sleep -Milliseconds 1500
        $after = [UiDrive]::SendMessage($list, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero)
        Write-Output ("записей до вопроса: {0}, после Enter: {1}" -f $before, $after)
        Save-Window $main $Out
    }

    'simple-view' {
        # Простой вид (лента): строка меню остаётся, столбцов два, а окно можно
        # вытянуть в узкую вертикальную полосу — и у ленты своя геометрия окна,
        # так что полный вид возвращается в своём размере.
        [UiDrive]::Bring($main)
        Write-Output ("полный вид: окно {0}" -f (Get-WindowSize $main))
        [UiDrive]::Command($main, 217, 0)   # Вид → Простой вид (лента)
        Start-Sleep -Milliseconds 1500
        Write-Output ("лента: окно {0}" -f (Get-WindowSize $main))
        # 0x0014 = SWP_NOZORDER | SWP_NOACTIVATE
        [UiDrive]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, 300, 820, 0x0014) | Out-Null
        Start-Sleep -Milliseconds 900
        Write-Output ("лента сжата до {0}" -f (Get-WindowSize $main))
        Save-Window $main "$Out-feed"
        # Ниже минимума ленты окно не идёт: попытка сжать до 120x150 ничего не даёт.
        [UiDrive]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, 120, 150, 0x0014) | Out-Null
        Start-Sleep -Milliseconds 600
        Write-Output ("попытка сжать до 120x150 дала {0}" -f (Get-WindowSize $main))
        [UiDrive]::Command($main, 217, 0)   # обратно в полный вид
        Start-Sleep -Milliseconds 1500
        Write-Output ("полный вид вернулся: окно {0}" -f (Get-WindowSize $main))
        Save-Window $main "$Out-full"
    }

    'theme-dark' {
        # Ночная тема (261): тёмными становятся окна, список, поля, заголовок
        # таблицы и заголовок окна.  Красным тема не ставится — так же, как и
        # наоборот: обе команды идут тем же путём, что и щелчок в меню.
        [UiDrive]::Bring($main)
        [UiDrive]::Command($main, 261, 0)
        Start-Sleep -Milliseconds 1200
        Write-Output ("ночная тема: окно {0}" -f (Get-WindowSize $main))
        Save-Window $main "$Out-dark"
    }

    'theme-light' {
        # Дневная тема (260) возвращает системные цвета.
        [UiDrive]::Command($main, 260, 0)
        Start-Sleep -Milliseconds 1000
        Save-Window $main "$Out-light"
    }

    'feed-time-column' {
        # Столбец времени в ленте (240): тем же пунктом он и выключается, так что
        # команду можно подать дважды.
        [UiDrive]::Command($main, 240, 0)
        Start-Sleep -Milliseconds 900
        Write-Output ("лента со столбцом времени: окно {0}" -f (Get-WindowSize $main))
        Save-Window $main "$Out-column"
    }

    'feed-three-lines' {
        # Строки ленты: 251 — две строки, 252 — три.
        [UiDrive]::Command($main, 252, 0)
        Start-Sleep -Milliseconds 900
        Write-Output ("лента в три строки: окно {0}" -f (Get-WindowSize $main))
        Save-Window $main "$Out-three-lines"
    }

    'quit' {
        # Выход через меню «Файл → Выход»: приложение сохраняет настройки и
        # закрывает журнал так же, как это делает человек.
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]207, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1500
    }

    'settings' {
        [UiDrive]::Bring($main)
        [UiDrive]::PostMessage($main, 0x0111, [IntPtr]208, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 1500
        $settings = Get-AppWindow 'WinNotificationLogSettings'
        $rect = New-Object UiDrive+RECT
        [UiDrive]::GetWindowRect($settings, [ref]$rect) | Out-Null
        Write-Output "settings window: $($rect.Left),$($rect.Top) - $($rect.Right),$($rect.Bottom)"
        Save-Window $settings $Out
        [UiDrive]::Escape()
        Start-Sleep -Milliseconds 400
    }
}
