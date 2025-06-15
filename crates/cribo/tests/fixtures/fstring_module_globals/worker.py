"""Worker module with globals that need lifting when using f-strings"""

# Module-level variables that will be referenced with global
status = "idle"
counter = 0
tasks = []


class Worker:
    """Worker class that uses global variables in f-strings"""

    def __init__(self):
        self.name = "Worker1"

    def process(self, data):
        """Process data and update global state"""
        global status, counter
        status = "processing"
        counter += 1
        # This f-string uses globals that need to be lifted
        return f"Processing {data}: status={status}, count={counter}"

    def get_status(self):
        """Get current status using f-string with globals"""
        global status, counter, tasks
        # Complex f-string with multiple global references
        return f"Worker {self.name}: status='{status}', processed={counter}, pending={len(tasks)}"

    def do_work(self):
        """Do some work and update globals"""
        global status, counter, tasks
        tasks.append(f"Task {counter + 1}")
        status = "working"
        counter += 1
        # Nested f-string expressions with globals
        return f"Started task: {tasks[-1]} (total: {len(tasks)}, status: {status.upper()})"
