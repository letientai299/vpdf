"""Place the benchmark window through Kitty."""

from kittens.tui.handler import result_handler
from kitty.fast_data_types import set_os_window_pos, toggle_os_window_visibility


def main(args):
    pass


@result_handler(no_ui=True)
def handle_result(args, data, target_window_id, boss):
    window = boss.window_id_map[target_window_id]
    set_os_window_pos(window.os_window_id, *map(int, args[1:]))
    toggle_os_window_visibility(window.os_window_id, True)
