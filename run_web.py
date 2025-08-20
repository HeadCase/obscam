#!/usr/bin/env python3
"""
Simple script to start the ObsCam web interface.

This starts both:
- Flask web server on port 5000 (main webpage)
- FastAPI server on port 8000 (API endpoints)
"""

if __name__ == "__main__":
    from src.obscam.web import start_web_servers
    start_web_servers()