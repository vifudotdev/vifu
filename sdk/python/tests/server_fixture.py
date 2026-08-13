import signal
import time


running = True


def stop(_signal, _frame):
    global running
    running = False


signal.signal(signal.SIGTERM, stop)
print("fixture ready", flush=True)
while running:
    time.sleep(0.01)
