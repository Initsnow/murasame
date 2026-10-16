import os
import sys
import time
import subprocess
import ctypes
from ctypes import wintypes
from PIL import Image

user32 = ctypes.windll.user32
gdi32 = ctypes.windll.gdi32

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2) # Per monitor DPI aware
except Exception:
    try:
        user32.SetProcessDPIAware()
    except Exception:
        pass


def find_window_by_title(title_sub):
    found = []
    def enum_windows_proc(hwnd, lParam):
        if user32.IsWindowVisible(hwnd):
            length = user32.GetWindowTextLengthW(hwnd)
            if length > 0:
                buff = ctypes.create_unicode_buffer(length + 1)
                user32.GetWindowTextW(hwnd, buff, length + 1)
                if title_sub in buff.value:
                    found.append((hwnd, buff.value))
        return True
    
    WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows(WNDENUMPROC(enum_windows_proc), 0)
    return found

def capture_hwnd_to_file(hwnd, path):
    rect = wintypes.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    w = rect.right - rect.left
    h = rect.bottom - rect.top
    if w <= 0 or h <= 0:
        return False
    
    hwnd_dc = user32.GetWindowDC(hwnd)
    mem_dc = gdi32.CreateCompatibleDC(hwnd_dc)
    bitmap = gdi32.CreateCompatibleBitmap(hwnd_dc, w, h)
    gdi32.SelectObject(mem_dc, bitmap)
    
    res = user32.PrintWindow(hwnd, mem_dc, 2)
    if not res:
        gdi32.BitBlt(mem_dc, 0, 0, w, h, hwnd_dc, 0, 0, 0x00CC0020)
    
    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [
            ('biSize', wintypes.DWORD),
            ('biWidth', wintypes.LONG),
            ('biHeight', wintypes.LONG),
            ('biPlanes', wintypes.WORD),
            ('biBitCount', wintypes.WORD),
            ('biCompression', wintypes.DWORD),
            ('biSizeImage', wintypes.DWORD),
            ('biXPelsPerMeter', wintypes.LONG),
            ('biYPelsPerMeter', wintypes.LONG),
            ('biClrUsed', wintypes.DWORD),
            ('biClrImportant', wintypes.DWORD)
        ]
    
    bmi = BITMAPINFOHEADER()
    bmi.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.biWidth = w
    bmi.biHeight = -h
    bmi.biPlanes = 1
    bmi.biBitCount = 32
    bmi.biCompression = 0
    
    buf = (ctypes.c_char * (w * h * 4))()
    gdi32.GetDIBits(mem_dc, bitmap, 0, h, ctypes.byref(buf), ctypes.byref(bmi), 0)
    
    im = Image.frombuffer('RGBA', (w, h), buf, 'raw', 'BGRA', 0, 1)
    im.save(path)
    
    gdi32.DeleteObject(bitmap)
    gdi32.DeleteDC(mem_dc)
    user32.ReleaseDC(hwnd, hwnd_dc)
    return True

def capture_tab(tab_idx, output_path):
    print(f"Launching murasame.exe --settings --tab {tab_idx}...")
    proc = subprocess.Popen([r"target\debug\murasame.exe", "--settings", "--tab", str(tab_idx)], cwd=os.getcwd())
    time.sleep(3.5)
    try:
        settings_wins = find_window_by_title("桌宠 设置")
        if settings_wins:
            shwnd = settings_wins[0][0]
            ok = capture_hwnd_to_file(shwnd, output_path)
            print(f"Captured {output_path}: {ok}")
    finally:
        proc.terminate()
        try: proc.wait(timeout=2)
        except: proc.kill()

def main():
    os.makedirs("test_screenshots", exist_ok=True)
    capture_tab(0, "test_screenshots/settings_tts_tab.png")
    capture_tab(1, "test_screenshots/settings_ai_tab.png")
    capture_tab(2, "test_screenshots/settings_live2d_tab.png")
    capture_tab(3, "test_screenshots/settings_subtitles_tab.png")
    capture_tab(4, "test_screenshots/settings_user_tab.png")
    print("All screenshots captured!")

if __name__ == "__main__":
    main()
