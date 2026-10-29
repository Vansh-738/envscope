import os
from os import getenv

DATABASE_URL = os.environ["DATABASE_URL"]
LOG_LEVEL = getenv("LOG_LEVEL", "info")
PORT = os.environ.get("PORT", "8000")
