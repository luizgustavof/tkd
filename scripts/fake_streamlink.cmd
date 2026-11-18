@echo off
rem streamlink falso dos experimentos de caos (ver fake_streamlink.py)
"%~dp0..\tools\rea-video\.venv\Scripts\python.exe" -I "%~dp0fake_streamlink.py" %*
