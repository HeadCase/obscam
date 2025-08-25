# ObsCam - Observatory Camera Monitoring

High-performance camera monitoring system optimized for Raspberry Pi observatory environments.

## Features

### Core Functionality
- **Real-time camera feed** with sub-500ms frame updates for telescope monitoring
- **Dual capture modes**: Fast video monitoring and long exposure snapshots
- **Multi-client support** with master/observer role management
- **WebSocket streaming** for low-latency observatory monitoring

### Reliability & Performance
- **USB camera recovery** - Automatic reconnection from hardware failures
- **Connection limiting** - Pi resource protection (max 5 concurrent connections)
- **Session persistence** - VPN-optimized 30-minute timeouts
- **Memory efficient** - Reduced cleanup frequency for Pi constraints

### Production Logging (New!)
- **Structured logging** with loguru for production debugging
- **Pi-optimized storage** - 10MB rotation, 7-day retention, compression
- **Remote access** - Logs saved to `logs` for SSH access
- **Event tracking** - USB recovery, session management, WebSocket errors
- **Performance metrics** - Frame rates, response times, resource usage

## Quick Start

### Installation
```bash
# Install dependencies
uv sync

# Run application
uv run obscam
```

### Access Points
- **Web Interface**: http://localhost:5000
- **API Endpoints**: http://localhost:8000
- **Force Master Mode**: http://localhost:5000/?force_master=true

### Environment Variables
```bash
# Enable detailed debug logging (optional)
export OBSCAM_DEBUG=true
```

## Production Deployment

### Log Monitoring
```bash
# View live logs
tail -f logs/obscam.log

# Monitor errors only
tail -f logs/obscam-error.log

# Remote log access via SSH
ssh pi@your-observatory "tail -f logs/obscam.log"

# Download logs for analysis
scp pi@your-observatory:logs/*.log ./local-logs/
```

### Log Files
- `logs/obscam.log` - Main application log (INFO+)
- `logs/obscam-error.log` - Error-only log for quick issue identification  
- `logs/obscam-debug.log` - Detailed debug log (debug mode only)

### Automatic Log Rotation
- **Main/Error logs**: 10MB rotation, 7-14 day retention
- **Debug logs**: 20MB rotation, 3-day retention  
- **Compression**: gzip enabled to conserve Pi storage
- **Cleanup**: Automatic archived log removal

## Development

### Testing
```bash
# Test reliability features
python test_reliability.py

# Test logging implementation
python test_logging.py
```

### Architecture
Built following Pi-specific design principles:
- **REUSE over CREATE** - Leverage existing Flask/FastAPI threads
- **MEMORY over DISK** - Keep state in RAM, not databases
- **CLIENT over SERVER** - Push complexity to desktop browsers
- **SIMPLE over FEATURE-RICH** - Observatory needs reliability over features
- **GRACEFUL DEGRADATION** - Lower quality beats service failure

## Troubleshooting

### Common Issues

**Camera not connecting:**
```bash
# Check logs for USB errors
grep "usb_failure" logs/obscam.log

# Manual reconnection attempt
curl http://localhost:8000/api/connect
```

**WebSocket connection failures:**
```bash
# Monitor WebSocket events
grep "WebSocket" logs/obscam.log

# Check connection limits
curl http://localhost:8000/api/health
```

**Session management issues:**
```bash
# View session events
grep "session_event" logs/obscam.log

# Force master transfer
curl -X POST http://localhost:8000/api/force-master/SESSION_ID
```

### Performance Debugging
```bash
# Monitor performance metrics
grep "Performance" logs/obscam.log

# View resource usage
grep "memory\|cpu" logs/obscam.log
```

## Hardware Requirements

- **Raspberry Pi 4** (recommended) or Pi 3B+
- **ZWO ASI camera** (ASI662MC tested)  
- **Stable network** for remote monitoring
- **microSD card** (32GB+ recommended for log storage)

## License

MIT License - Built for observatory automation and remote telescope monitoring.
