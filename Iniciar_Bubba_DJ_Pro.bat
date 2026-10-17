@echo off
rem Bubba DJ Pro v0.4.0 - lanzador
rem Puedes pasar pistas como argumentos: la primera va al Deck 1 y la segunda al Deck 2.
cd /d "%~dp0"
start "" "%~dp0BubbaDJPro.exe" %*
exit
