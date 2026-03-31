use luars::{FromLua, IntoLua, LuaResult, LuaVM};

pub(crate) fn into_single_value<T: IntoLua>(
    vm: &mut LuaVM,
    value: T,
    api_name: &str,
) -> LuaResult<luars::LuaValue> {
    let mut values = collect_values(vm, value)?;
    if values.len() != 1 {
        return Err(vm.error(format!(
            "{} expects exactly one Lua value, got {}",
            api_name,
            values.len()
        )));
    }
    Ok(values.pop().unwrap())
}

pub(crate) fn collect_values<T: IntoLua>(
    vm: &mut LuaVM,
    value: T,
) -> LuaResult<Vec<luars::LuaValue>> {
    let base_top = vm.main_state().get_top();

    let pushed = {
        let state = vm.main_state();
        match value.into_lua(state) {
            Ok(pushed) => pushed,
            Err(err) => {
                state.set_top_raw(base_top);
                return Err(state.error(err));
            }
        }
    };

    let mut values = Vec::with_capacity(pushed);
    {
        let state = vm.main_state();
        for index in base_top..base_top + pushed {
            let Some(value) = state.stack_get(index) else {
                state.set_top_raw(base_top);
                return Err(state.error(
                    "internal error: failed to collect Lua values from stack".to_owned(),
                ));
            };
            values.push(value);
        }
        state.set_top_raw(base_top);
    }

    Ok(values)
}

pub(crate) fn from_value<T: FromLua>(
    vm: &mut LuaVM,
    value: luars::LuaValue,
    api_name: &str,
) -> LuaResult<T> {
    T::from_lua(value, vm.main_state()).map_err(|msg| vm.error(format!("{}: {}", api_name, msg)))
}