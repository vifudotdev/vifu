from vifu import Vifu
from vifu.integrations.foundry import FoundryLocal


def main() -> None:
    app = Vifu("Foundry Local Chat", capture_trace_content=True)
    app.agent("chat", FoundryLocal("qwen2.5-0.5b"))
    app.run()


if __name__ == "__main__":
    main()
