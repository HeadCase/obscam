#!/usr/bin/env python3
"""Simple script to start the ObsCam web interface.

This starts both:
- Flask web server on port 5000 (main webpage)
- FastAPI server on port 8000 (API endpoints)
"""

from src.obscam.web import start_web_servers

if __name__ == "__main__":
    start_web_servers()

