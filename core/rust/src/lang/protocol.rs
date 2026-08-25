#[path = "protocol/declarations.rs"]
pub(crate) mod declarations;
#[path = "protocol/iabort.rs"]
pub mod iabort;
#[path = "protocol/iapplicable.rs"]
pub mod iapplicable;
#[path = "protocol/iassoc.rs"]
pub mod iassoc;
#[path = "protocol/icas.rs"]
pub mod icas;
#[path = "protocol/iclose.rs"]
pub mod iclose;
#[path = "protocol/iclosed.rs"]
pub mod iclosed;
#[path = "protocol/icoll.rs"]
pub mod icoll;
#[path = "protocol/icomponent.rs"]
pub mod icomponent;
#[path = "protocol/iconj.rs"]
pub mod iconj;
#[path = "protocol/icons.rs"]
pub mod icons;
#[path = "protocol/icontext.rs"]
pub mod icontext;
#[path = "protocol/icontextlifecycle.rs"]
pub mod icontextlifecycle;
#[path = "protocol/icoroutine.rs"]
pub mod icoroutine;
#[path = "protocol/icount.rs"]
pub mod icount;
#[path = "protocol/ideps.rs"]
pub mod ideps;
#[path = "protocol/ideref.rs"]
pub mod ideref;
#[path = "protocol/idereftimeout.rs"]
pub mod idereftimeout;
#[path = "protocol/idisplay.rs"]
pub mod idisplay;
#[path = "protocol/idissoc.rs"]
pub mod idissoc;
#[path = "protocol/iempty.rs"]
pub mod iempty;
#[path = "protocol/iencodable.rs"]
pub mod iencodable;
#[path = "protocol/iencode.rs"]
pub mod iencode;
#[path = "protocol/iencodevisitor.rs"]
pub mod iencodevisitor;
#[path = "protocol/iequality.rs"]
pub mod iequality;
#[path = "protocol/iexinfo.rs"]
pub mod iexinfo;
#[path = "protocol/ifind.rs"]
pub mod ifind;
#[path = "protocol/iflush.rs"]
pub mod iflush;
#[path = "protocol/ifn.rs"]
pub mod ifn;
#[path = "protocol/ihash.rs"]
pub mod ihash;
#[path = "protocol/ihashcached.rs"]
pub mod ihashcached;
#[path = "protocol/iindexed.rs"]
pub mod iindexed;
#[path = "protocol/iindexedkv.rs"]
pub mod iindexedkv;
#[path = "protocol/iinvokein.rs"]
pub mod iinvokein;
#[path = "protocol/imaptype.rs"]
pub mod imaptype;
#[path = "protocol/iiter.rs"]
pub mod iiter;
#[path = "protocol/iiterator.rs"]
pub mod iiterator;
#[path = "protocol/ilineartype.rs"]
pub mod ilineartype;
#[path = "protocol/isequential.rs"]
pub mod isequential;
#[path = "protocol/ilookup.rs"]
pub mod ilookup;
#[path = "protocol/imatch.rs"]
pub mod imatch;
#[path = "protocol/imetadata.rs"]
pub mod imetadata;
#[path = "protocol/imutable.rs"]
pub mod imutable;
#[path = "protocol/inamespaced.rs"]
pub mod inamespaced;
#[path = "protocol/inth.rs"]
pub mod inth;
#[path = "protocol/iobjtype.rs"]
pub mod iobjtype;
#[path = "protocol/iofn.rs"]
pub mod iofn;
#[path = "protocol/ipair.rs"]
pub mod ipair;
#[path = "protocol/ipeekfirst.rs"]
pub mod ipeekfirst;
#[path = "protocol/ipeeklast.rs"]
pub mod ipeeklast;
#[path = "protocol/ipersistent.rs"]
pub mod ipersistent;
#[path = "protocol/ipointer.rs"]
pub mod ipointer;
#[path = "protocol/ipopfirst.rs"]
pub mod ipopfirst;
#[path = "protocol/ipoplast.rs"]
pub mod ipoplast;
#[path = "protocol/ipromise.rs"]
pub mod ipromise;
#[path = "protocol/ipushfirst.rs"]
pub mod ipushfirst;
#[path = "protocol/ipushlast.rs"]
pub mod ipushlast;
#[path = "protocol/irealize.rs"]
pub mod irealize;
#[path = "protocol/ireduce.rs"]
pub mod ireduce;
#[path = "protocol/ireset.rs"]
pub mod ireset;
#[path = "protocol/ispace.rs"]
pub mod ispace;
#[path = "protocol/istream.rs"]
pub mod istream;
#[path = "protocol/istreamduplex.rs"]
pub mod istreamduplex;
#[path = "protocol/istreamoffer.rs"]
pub mod istreamoffer;
#[path = "protocol/istreampoll.rs"]
pub mod istreampoll;
#[path = "protocol/istreamwrite.rs"]
pub mod istreamwrite;
#[path = "protocol/istringlike.rs"]
pub mod istringlike;
#[path = "protocol/isettype.rs"]
pub mod isettype;
#[path = "protocol/itomutable.rs"]
pub mod itomutable;
#[path = "protocol/itopersistent.rs"]
pub mod itopersistent;
#[path = "protocol/iwatch.rs"]
pub mod iwatch;
#[path = "protocol/iwork.rs"]
pub mod iwork;
#[path = "protocol/iworkexecutor.rs"]
pub mod iworkexecutor;
#[path = "protocol/iworkhost.rs"]
pub mod iworkhost;
#[path = "protocol/iworkref.rs"]
pub mod iworkref;
#[path = "protocol/iworkrun.rs"]
pub mod iworkrun;
#[path = "protocol/iworkstore.rs"]
pub mod iworkstore;

pub use declarations::{
    find_protocol, protocol_declarations, ProtocolArity, ProtocolAvailability, ProtocolDeclaration,
    ProtocolMethodDeclaration,
};
pub use iabort::IAbort;
pub use iapplicable::IApplicable;
pub use iassoc::IAssoc;
pub use icas::ICas;
pub use iclose::IClose;
pub use iclosed::IClosed;
pub use icoll::IColl;
pub use icomponent::IComponent;
pub use iconj::IConj;
pub use icons::ICons;
pub use icontext::IContext;
pub use icontextlifecycle::IContextLifeCycle;
pub use icoroutine::ICoroutine;
pub use icount::ICount;
pub use ideps::IDeps;
pub use ideref::IDeref;
pub use idereftimeout::IDerefTimeout;
pub use idisplay::IDisplay;
pub use idissoc::IDissoc;
pub use iempty::IEmpty;
pub use iencodable::IEncodable;
pub use iencode::IEncode;
pub use iencodevisitor::IEncodeVisitor;
pub use iequality::IEquality;
pub use iexinfo::IExInfo;
pub use ifind::IFind;
pub use iflush::IFlush;
pub use ifn::IFn;
pub use ihash::{HashType, IHash};
pub use ihashcached::IHashCached;
pub use iindexed::IIndexed;
pub use iindexedkv::IIndexedKV;
pub use iinvokein::IInvokeIn;
pub use imaptype::IMapType;
pub use iiter::IIter;
pub use iiterator::IIterator;
pub use ilineartype::ILinearType;
pub use isequential::ISequential;
pub use ilookup::ILookup;
pub use imatch::IMatch;
pub use imetadata::{IMetadata, MetaType};
pub use imutable::IMutable;
pub use inamespaced::INamespaced;
pub use inth::INth;
pub use iobjtype::{IObjType, ObjType};
pub use iofn::IOFn;
pub use ipair::IPair;
pub use ipeekfirst::IPeekFirst;
pub use ipeeklast::IPeekLast;
pub use ipersistent::IPersistent;
pub use ipointer::IPointer;
pub use ipopfirst::IPopFirst;
pub use ipoplast::IPopLast;
pub use ipromise::IPromise;
pub use ipushfirst::IPushFirst;
pub use ipushlast::IPushLast;
pub use irealize::IRealize;
pub use ireduce::IReduce;
pub use ireset::IReset;
pub use isettype::ISetType;
pub use ispace::ISpace;
pub use istream::IStream;
pub use istreamduplex::IStreamDuplex;
pub use istreamoffer::IStreamOffer;
pub use istreampoll::IStreamPoll;
pub use istreamwrite::IStreamWrite;
pub use istringlike::IStringLike;
pub use itomutable::IToMutable;
pub use itopersistent::IToPersistent;
pub use iwatch::IWatch;
pub use iwork::IWork;
pub use iworkexecutor::IWorkExecutor;
pub use iworkhost::IWorkHost;
pub use iworkref::IWorkRef;
pub use iworkrun::IWorkRun;
pub use iworkstore::IWorkStore;

#[cfg(test)]
mod tests {
    use super::{IFind, IObjType, ObjType};
    use crate::lang::data::{Cons, List, Queue, Seq, Tuple, Vector};

    struct Entries(Vec<(u8, Option<u8>)>);

    impl IFind<u8> for Entries {
        type Output = (u8, Option<u8>);

        fn find(&self, key: &u8) -> Option<Self::Output> {
            self.0
                .iter()
                .find(|(candidate, _)| candidate == key)
                .cloned()
        }
    }

    #[test]
    fn sequential_family_uses_java_protocol_category() {
        assert_eq!(List::<i32>::new().obj_type(), ObjType::Sequential);
        assert_eq!(Vector::<i32>::new().obj_type(), ObjType::Sequential);
        assert_eq!(Tuple::<i32>::Tup0.obj_type(), ObjType::Sequential);
        assert_eq!(Queue::<i32>::new().obj_type(), ObjType::Sequential);
        assert_eq!(Cons::new(1, List::new()).obj_type(), ObjType::Sequential);
        assert_eq!(Seq::new([1].into_iter()).obj_type(), ObjType::Sequential);
    }

    #[test]
    fn find_has_distinguishes_absence_from_a_nil_value() {
        let entries = Entries(vec![(1, None), (2, Some(7))]);
        assert_eq!(entries.find(&1), Some((1, None)));
        assert!(entries.has(&1));
        assert!(entries.has(&2));
        assert!(!entries.has(&3));
    }
}
