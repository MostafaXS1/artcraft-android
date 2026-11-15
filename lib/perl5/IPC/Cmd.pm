package IPC::Cmd;
use strict;
use warnings;
use vars qw($VERSION @ISA @EXPORT_OK);
$VERSION = "0.96";
require Exporter;
@ISA = qw(Exporter);
@EXPORT_OK = qw(can_run run);

sub can_run {
    my $command = shift;
    return $command if -x $command && ! -d $command;
    for my $dir (split /:/, ($ENV{PATH} || "")) {
        my $full = "$dir/$command";
        return $full if -x $full && ! -d $full;
    }
    return undef;
}

sub run {
    my %args = @_;
    my $cmd = $args{command};
    my $rc = system($cmd);
    return ($rc == 0, $rc, [], [], []);
}

1;
