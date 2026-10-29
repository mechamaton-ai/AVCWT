#!/bin/bash
cd "$(dirname "$0")"
if [ ! -d "venv" ]; then
    python3 -m venv venv
fi
source venv/bin/activate
pip install -r requirements.txt
pyinstaller --noconfirm --onedir --windowed --name "AVCWT" "client.py"
echo "Build complete! Check the 'dist' folder."
