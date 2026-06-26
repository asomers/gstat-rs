#! /bin/sh

CRATEDIR=`dirname $0`/..
OUTFILE=$1
if [ -z "$OUTFILE" ]; then
	case $(uname -m) in
	i386)
		OUTFILE=${CRATEDIR}/src/ffi32.rs
		;;
	armv7)
		OUTFILE=${CRATEDIR}/src/ffi32.rs
		;;
	*)
		OUTFILE=${CRATEDIR}/src/ffi64.rs
		;;
	esac
fi

bindgen --generate functions,types,vars \
	--allowlist-function 'geom_.*' \
	--allowlist-function 'gctl_.*' \
	--allowlist-function 'g_.*' \
	--allowlist-type 'devstat_trans_flags' \
	${CRATEDIR}/bindgen/wrapper.h > ${OUTFILE}
