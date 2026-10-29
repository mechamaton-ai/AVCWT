#!/bin/bash
cd "$(dirname "$0")"
pyinstaller --noconfirm --windowed client.py