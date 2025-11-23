// Lua-Rust异步桥接
// 允许Lua协程直接调用Rust的async函数
// 使用tokio作为异步运行时

use crate::LuaValue;
use crate::lua_vm::{LuaError, LuaResult, LuaVM};
use crate::lua_value::MultiValue;
use std::future::Future;
use std::pin::Pin;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

/// 异步任务的唯一标识符
pub type AsyncTaskId = u64;

/// Rust异步函数的类型
/// 接受参数Vec<LuaValue>，返回Future
pub type AsyncFn = Arc<dyn Fn(Vec<LuaValue>) -> Pin<Box<dyn Future<Output = LuaResult<Vec<LuaValue>>> + Send>> + Send + Sync>;

/// 异步任务状态
#[derive(Debug, Clone)]
pub enum AsyncTaskState {
    /// 任务正在执行
    Running,
    /// 任务已完成
    Completed(LuaResult<Vec<LuaValue>>),
    /// 任务被取消
    Cancelled,
}

/// 异步任务信息
pub struct AsyncTask {
    /// 任务ID
    pub id: AsyncTaskId,
    /// 关联的协程LuaValue
    pub coroutine: LuaValue,
    /// 任务状态
    pub state: AsyncTaskState,
}

/// 异步执行器
/// 管理所有异步任务，使用tokio runtime执行Future
pub struct AsyncExecutor {
    /// Tokio运行时
    runtime: Runtime,
    /// 任务ID生成器
    next_task_id: AsyncTaskId,
    /// 活跃的异步任务
    tasks: Arc<Mutex<HashMap<AsyncTaskId, AsyncTask>>>,
    /// 已注册的异步函数
    async_functions: HashMap<String, AsyncFn>,
}

impl AsyncExecutor {
    pub fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        AsyncExecutor {
            runtime,
            next_task_id: 1,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            async_functions: HashMap::new(),
        }
    }

    /// 注册一个异步函数
    pub fn register_async_function<F, Fut>(&mut self, name: String, func: F)
    where
        F: Fn(Vec<LuaValue>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = LuaResult<Vec<LuaValue>>> + Send + 'static,
    {
        let wrapped: AsyncFn = Arc::new(move |args| Box::pin(func(args)));
        self.async_functions.insert(name, wrapped);
    }

    /// 启动一个异步任务
    /// 返回任务ID
    pub fn spawn_task(
        &mut self,
        func_name: &str,
        args: Vec<LuaValue>,
        coroutine: LuaValue,
    ) -> Result<AsyncTaskId, LuaError> {
        let func = self.async_functions.get(func_name)
            .ok_or_else(|| LuaError::RuntimeError(format!("Async function '{}' not registered", func_name)))?
            .clone();

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let task = AsyncTask {
            id: task_id,
            coroutine: coroutine.clone(),
            state: AsyncTaskState::Running,
        };

        {
            let mut tasks = self.tasks.lock().unwrap();
            tasks.insert(task_id, task);
        }

        // 在tokio runtime中执行异步任务
        let tasks_clone = Arc::clone(&self.tasks);
        self.runtime.spawn(async move {
            let result = func(args).await;
            
            // 更新任务状态为完成
            let mut tasks = tasks_clone.lock().unwrap();
            if let Some(task) = tasks.get_mut(&task_id) {
                task.state = AsyncTaskState::Completed(result);
            }
        });

        Ok(task_id)
    }

    /// 检查并收集已完成的任务
    /// 返回已完成的任务列表（task_id, coroutine, result）
    pub fn collect_completed_tasks(&mut self) -> Vec<(AsyncTaskId, LuaValue, LuaResult<Vec<LuaValue>>)> {
        let mut completed = Vec::new();
        let mut tasks = self.tasks.lock().unwrap();
        let mut to_remove = Vec::new();

        for (task_id, task) in tasks.iter() {
            match &task.state {
                AsyncTaskState::Completed(result) => {
                    completed.push((*task_id, task.coroutine, result.clone()));
                    to_remove.push(*task_id);
                }
                _ => {}
            }
        }

        // 移除已完成的任务
        for task_id in to_remove {
            tasks.remove(&task_id);
        }

        completed
    }

    /// 获取异步函数名列表
    pub fn get_function_names(&self) -> Vec<&str> {
        self.async_functions.keys().map(|s| s.as_str()).collect()
    }

    /// 检查是否注册了某个异步函数
    pub fn has_function(&self, name: &str) -> bool {
        self.async_functions.contains_key(name)
    }

    /// 获取活跃任务数量
    pub fn active_task_count(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }
}

/// 创建一个包装器CFunction，用于注册async函数到Lua
/// 当调用时，它会启动async任务并yield当前协程
pub fn create_async_wrapper(
    func_name: String,
) -> impl Fn(&mut LuaVM) -> LuaResult<MultiValue> {
    move |vm: &mut LuaVM| -> LuaResult<MultiValue> {
        // 检查是否在协程中
        let coroutine = vm.current_thread_value.clone()
            .ok_or_else(|| LuaError::RuntimeError(
                format!("async function '{}' can only be called from within a coroutine", func_name)
            ))?;

        // 收集参数
        let frame = vm.frames.last().unwrap();
        let base = frame.base_ptr;
        let top = frame.top;
        let mut args = Vec::new();
        for i in 1..top {
            args.push(vm.register_stack[base + i]);
        }

        // 启动异步任务
        let task_id = vm.async_executor.spawn_task(&func_name, args, coroutine)?;
        
        // Yield协程，返回task_id
        Err(LuaError::Yield(vec![LuaValue::integer(task_id as i64)]))
    }
}