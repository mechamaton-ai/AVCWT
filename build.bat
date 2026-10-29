@echo off
cd /d "%~dp0"
if not exist venv (
    python -m venv venv
)
call venv\Scripts\activate.bat
pip install -r requirements.txt
pyinstaller --noconfirm --onedir --windowed --name "AVCWT" "client.py"
echo Build complete! Check the 'dist' folder.
pause
