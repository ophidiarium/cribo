# Services package
from .auth import User as ServiceUser

# More conflicts at package level
process = lambda x, y: f"services_process: {x}, {y}"
validate = "services_validate"
