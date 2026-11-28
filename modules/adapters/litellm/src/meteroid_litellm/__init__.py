"""Bill LiteLLM usage with Meteroid. In the proxy's config: `callbacks: ["meteroid_litellm.handler"]`."""

__version__ = "0.1.0"

from .events import events_for  # noqa: E402
from .logger import MeteroidLogger  # noqa: E402
from .settings import Settings  # noqa: E402

handler = MeteroidLogger()

__all__ = ["MeteroidLogger", "Settings", "events_for", "handler"]
