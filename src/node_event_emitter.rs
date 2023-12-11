
///
pub trait NodeEventEmitter  {
    fn on_event(&mut self, e: NodeEvent);
}
