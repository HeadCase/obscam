import time
import uuid
import threading
from typing import Dict, Optional
from dataclasses import dataclass, field
from fastapi import WebSocket

from .logging_config import get_logger, log_session_event

logger = get_logger("session_manager")


@dataclass
class ClientSession:
    """Represents a connected client session."""

    session_id: str
    websocket: Optional[WebSocket] = None
    ip_address: str = ""
    is_master: bool = False
    connected_at: float = field(default_factory=time.time)
    last_activity: float = field(default_factory=time.time)
    last_heartbeat: float = field(default_factory=time.time)

    def update_activity(self):
        """Update the last activity timestamp."""
        self.last_activity = time.time()

    def update_heartbeat(self):
        """Update the heartbeat timestamp."""
        self.last_heartbeat = time.time()

    def is_active(self, timeout_seconds: int = 1800) -> bool:
        """Check if the session is still active (default 30 minutes for VPN reliability)."""
        return (time.time() - self.last_activity) < timeout_seconds

    def is_connected(self, heartbeat_timeout: int = 30) -> bool:
        """Check if WebSocket connection is alive (default 30 seconds)."""
        return (time.time() - self.last_heartbeat) < heartbeat_timeout


class SessionManager:
    """Manages client sessions and master/observer roles."""

    def __init__(self, max_connections: int = 5):
        self.sessions: Dict[str, ClientSession] = {}
        self.current_master: Optional[str] = None
        self.lock = threading.Lock()
        self.max_connections = max_connections  # VPN/Pi resource protection

        # Start cleanup thread
        self.cleanup_thread = threading.Thread(target=self._cleanup_loop, daemon=True)
        self.cleanup_thread.start()

    def create_session(
        self, websocket: WebSocket, ip_address: str, force_master: bool = False
    ) -> str:
        """Create a new client session."""
        with self.lock:
            # Check connection limit (Pi resource protection)
            active_sessions = [
                s for s in self.sessions.values() if s.is_active() and s.is_connected()
            ]
            if len(active_sessions) >= self.max_connections:
                raise ConnectionRefusedError(
                    f"Maximum connections ({self.max_connections}) reached"
                )

            session_id = str(uuid.uuid4())

            # Determine if this client should be master
            should_be_master = force_master or (self.current_master is None)

            session = ClientSession(
                session_id=session_id,
                websocket=websocket,
                ip_address=ip_address,
                is_master=should_be_master,
            )

            self.sessions[session_id] = session

            if should_be_master:
                self.current_master = session_id
                log_session_event(
                    "master_created",
                    session_id,
                    ip_address=ip_address,
                    total_sessions=len(self.sessions),
                )
            else:
                log_session_event(
                    "observer_created",
                    session_id,
                    ip_address=ip_address,
                    total_sessions=len(self.sessions),
                )

            return session_id

    def remove_session(self, session_id: str):
        """Remove a client session and handle master transition."""
        with self.lock:
            if session_id not in self.sessions:
                return

            session = self.sessions[session_id]
            was_master = session.is_master

            del self.sessions[session_id]
            log_session_event(
                "session_removed",
                session_id,
                ip_address=session.ip_address,
                was_master=was_master,
                remaining_sessions=len(self.sessions),
            )

            # If master disconnected, promote next client
            if was_master and self.current_master == session_id:
                self._elect_new_master()

    def get_session(self, session_id: str) -> Optional[ClientSession]:
        """Get a client session by ID."""
        return self.sessions.get(session_id)

    def get_master_session(self) -> Optional[ClientSession]:
        """Get the current master session."""
        if self.current_master:
            return self.sessions.get(self.current_master)
        return None

    def is_master(self, session_id: str) -> bool:
        """Check if a session is the current master."""
        return self.current_master == session_id

    def request_master_transfer(self, from_session_id: str, to_session_id: str) -> bool:
        """Request master transfer from one session to another."""
        with self.lock:
            # Validate both sessions exist
            if (
                from_session_id not in self.sessions
                or to_session_id not in self.sessions
            ):
                return False

            # Current master must approve (or be inactive)
            current_master = self.sessions.get(self.current_master)
            if current_master and current_master.is_active():
                # For now, auto-approve all requests
                # In future, could add approval workflow
                pass

            # Transfer master role
            self._set_master(to_session_id)
            return True

    def force_master_transfer(self, session_id: str) -> bool:
        """Force master transfer (admin override)."""
        with self.lock:
            if session_id not in self.sessions:
                return False

            self._set_master(session_id)
            log_session_event("force_master_transfer", session_id)
            return True

    def update_session_activity(self, session_id: str):
        """Update session activity timestamp."""
        session = self.sessions.get(session_id)
        if session:
            session.update_activity()

    def update_session_heartbeat(self, session_id: str):
        """Update session heartbeat timestamp."""
        session = self.sessions.get(session_id)
        if session:
            session.update_heartbeat()

    def get_session_info(self) -> dict:
        """Get information about all sessions."""
        with self.lock:
            master_session = self.get_master_session()

            return {
                "total_sessions": len(self.sessions),
                "master_session_id": self.current_master,
                "master_ip": master_session.ip_address if master_session else None,
                "observer_count": len(self.sessions)
                - (1 if self.current_master else 0),
                "sessions": [
                    {
                        "session_id": sid,
                        "ip_address": session.ip_address,
                        "is_master": session.is_master,
                        "connected_at": session.connected_at,
                        "last_activity": session.last_activity,
                        "is_active": session.is_active(),
                        "is_connected": session.is_connected(),
                    }
                    for sid, session in self.sessions.items()
                ],
            }

    def broadcast_to_observers(self, message: dict):
        """Send message to all observer sessions via WebSocket."""
        observer_sessions = [
            session
            for session in self.sessions.values()
            if not session.is_master and session.websocket
        ]

        for session in observer_sessions:
            try:
                # Note: This is async, would need proper async handling in real implementation
                # For now, this is a placeholder for the WebSocket broadcasting
                pass
            except Exception as e:
                print(f"Failed to broadcast to observer {session.session_id}: {e}")

    def _set_master(self, session_id: str):
        """Internal method to set a new master."""
        # Remove master role from current master
        if self.current_master and self.current_master in self.sessions:
            self.sessions[self.current_master].is_master = False

        # Set new master
        if session_id in self.sessions:
            self.sessions[session_id].is_master = True
            self.current_master = session_id
            log_session_event("master_transferred", session_id)

    def _elect_new_master(self):
        """Elect a new master from available sessions."""
        self.current_master = None

        # Find the oldest active session
        active_sessions = [
            (sid, session)
            for sid, session in self.sessions.items()
            if session.is_active() and session.is_connected()
        ]

        if active_sessions:
            # Sort by connection time (oldest first)
            active_sessions.sort(key=lambda x: x[1].connected_at)
            new_master_id = active_sessions[0][0]
            self._set_master(new_master_id)
        else:
            logger.warning("No active sessions available for master promotion")

    def _cleanup_loop(self):
        """Background thread to cleanup inactive sessions (VPN-optimized frequency)."""
        while True:
            try:
                time.sleep(300)  # Check every 5 minutes (reduce Pi CPU load)
                self._cleanup_inactive_sessions()
            except Exception as e:
                logger.error("Session cleanup error", error=str(e))

    def _cleanup_inactive_sessions(self):
        """Remove inactive or disconnected sessions."""
        with self.lock:
            inactive_sessions = [
                sid
                for sid, session in self.sessions.items()
                if not session.is_active() or not session.is_connected()
            ]

            for session_id in inactive_sessions:
                log_session_event("session_cleanup", session_id)
                self.remove_session(session_id)


# Global session manager instance
_session_manager = None


def get_session_manager() -> SessionManager:
    """Get the global session manager instance."""
    global _session_manager
    if _session_manager is None:
        _session_manager = SessionManager()
    return _session_manager
