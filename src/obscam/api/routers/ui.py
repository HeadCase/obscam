"""UI routes for obscam."""

from fastapi import APIRouter, Request
from fastapi.templating import Jinja2Templates

from obscam.common.constants import TEMPLATE_DIR

router = APIRouter()
templates = Jinja2Templates(directory=str(TEMPLATE_DIR))


@router.get("/")
async def index(request: Request):
    """Render the main page displaying the camera feed."""
    return templates.TemplateResponse(request, "index.html", {"request": request})
